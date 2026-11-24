//! Read-aloud speech synthesis using Windows' installed offline voices (B6-14, #599).
//!
//! Architecture & security guarantees (docs/architecture/read-aloud.md):
//! - Uses WinRT `Windows.Media.SpeechSynthesis.SpeechSynthesizer` and
//!   `Windows.Media.Playback.MediaPlayer` on a dedicated COM (MTA) worker thread, never on the
//!   main or WebView thread.
//! - Plain text only: calls `SynthesizeTextToStreamAsync`, NEVER `SynthesizeSsmlToStreamAsync`
//!   (SSML `<audio src>` / `<lexicon uri>` can reference external resources).
//! - Memory only: plays via `MediaSource::CreateFromStream`, NEVER `CreateFromUri` or temp files.
//! - Voice ids are matched by exact equality against `SpeechSynthesizer::AllVoices()`, never
//!   passed to filesystem or registry APIs.
//! - No logs, no temp files, and error messages never contain spoken text. Sensitive documents
//!   can be read aloud with zero persisted trace.

// COM initialization (`CoInitializeEx` / `CoUninitialize`) on the dedicated speech thread;
// each unsafe block documents why it is sound.
#![allow(unsafe_code)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use ipc_contract::limits::MAX_SPEECH_VOICES;
use ipc_contract::types::{ErrorCode, IpcError, SpeakArgs, SpeechEvent, SpeechVoice};
use ipc_contract::validate::Validate;
use windows::Foundation::Collections::IVectorChangedEventArgs;
use windows::Foundation::TypedEventHandler;
use windows::Media::Core::{MediaCueEventArgs, MediaSource, SpeechCue, TimedMetadataTrack};
use windows::Media::Playback::{
    MediaPlaybackItem, MediaPlaybackTimedMetadataTrackList, MediaPlayer,
    MediaPlayerFailedEventArgs, TimedMetadataTrackPresentationMode,
};
use windows::Media::SpeechSynthesis::{SpeechSynthesisStream, SpeechSynthesizer};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::{HSTRING, Interface};

/// Event sink for speech playback notifications.
pub type EventSink = Arc<dyn Fn(SpeechEvent) + Send + Sync>;

/// Failures returned by a [`SpeechBackend`]. Mapped to fixed static error messages so spoken
/// text can never leak into an error string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechError {
    NoVoices,
    NoAudioDevice,
    UnknownVoice,
    InvalidArgument,
    Failed,
}

impl SpeechError {
    pub fn into_ipc_error(self) -> IpcError {
        let (code, message) = match self {
            Self::NoVoices => (
                ErrorCode::SpeechNoVoices,
                "no offline speech voices installed",
            ),
            Self::NoAudioDevice => (
                ErrorCode::SpeechNoAudioDevice,
                "no audio playback device available",
            ),
            Self::UnknownVoice => (
                ErrorCode::SpeechUnknownVoice,
                "requested speech voice is not installed",
            ),
            Self::InvalidArgument => (ErrorCode::InvalidArgument, "invalid speech arguments"),
            Self::Failed => (ErrorCode::SpeechFailed, "speech playback failed"),
        };
        IpcError {
            code,
            message: message.to_owned(),
        }
    }
}

/// Speech synthesis and playback engine abstraction. Production uses [`WinRtSpeechBackend`];
/// unit tests inject a fake backend so no audio plays during tests.
pub trait SpeechBackend: Send + 'static {
    fn list_voices(&mut self) -> Result<Vec<SpeechVoice>, SpeechError>;
    fn speak(
        &mut self,
        text: &str,
        voice_id: Option<&str>,
        rate: f64,
        sink: EventSink,
    ) -> Result<(), SpeechError>;
    fn pause(&mut self) -> Result<(), SpeechError>;
    fn resume(&mut self) -> Result<(), SpeechError>;
    fn stop(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaybackState {
    Idle,
    Playing,
    Paused,
}

enum Command {
    ListVoices {
        reply: mpsc::SyncSender<Result<Vec<SpeechVoice>, SpeechError>>,
    },
    Speak {
        args: SpeakArgs,
        sink: EventSink,
        reply: mpsc::SyncSender<Result<(), SpeechError>>,
    },
    Pause {
        reply: mpsc::SyncSender<Result<(), SpeechError>>,
    },
    Resume {
        reply: mpsc::SyncSender<Result<(), SpeechError>>,
    },
    Stop {
        reply: mpsc::SyncSender<()>,
    },
    Shutdown,
}

/// Manages a single speech synthesis engine on its own thread.
pub struct SpeechService {
    tx: Mutex<Option<mpsc::Sender<Command>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl SpeechService {
    /// Starts the production WinRT speech service on a dedicated MTA COM thread.
    pub fn new_winrt() -> Self {
        Self::start(true, || Box::new(WinRtSpeechBackend::default()))
    }

    #[cfg(test)]
    fn with_backend<B: SpeechBackend>(backend: B) -> Self {
        Self::start(false, move || Box::new(backend))
    }

    fn start(
        init_com: bool,
        factory: impl FnOnce() -> Box<dyn SpeechBackend> + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("pdf-reader-speech".to_owned())
            .spawn(move || {
                let _com = init_com.then(ComGuard::init);
                let mut backend = factory();
                run_loop(&mut *backend, rx);
            })
            .expect("speech worker thread should spawn");
        Self {
            tx: Mutex::new(Some(tx)),
            worker: Mutex::new(Some(handle)),
        }
    }

    fn send(&self, cmd: Command) -> bool {
        let guard = self
            .tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref().is_some_and(|tx| tx.send(cmd).is_ok())
    }

    pub fn list_voices(&self) -> Result<Vec<SpeechVoice>, IpcError> {
        let (reply, rx) = mpsc::sync_channel(1);
        if !self.send(Command::ListVoices { reply }) {
            return Err(SpeechError::Failed.into_ipc_error());
        }
        rx.recv()
            .unwrap_or(Err(SpeechError::Failed))
            .map_err(SpeechError::into_ipc_error)
    }

    pub fn speak(&self, args: SpeakArgs, sink: EventSink) -> Result<(), IpcError> {
        // Validate before crossing to the worker thread; error never includes spoken text.
        if args.validate().is_err() {
            return Err(SpeechError::InvalidArgument.into_ipc_error());
        }
        let (reply, rx) = mpsc::sync_channel(1);
        if !self.send(Command::Speak { args, sink, reply }) {
            return Err(SpeechError::Failed.into_ipc_error());
        }
        rx.recv()
            .unwrap_or(Err(SpeechError::Failed))
            .map_err(SpeechError::into_ipc_error)
    }

    pub fn pause(&self) -> Result<(), IpcError> {
        let (reply, rx) = mpsc::sync_channel(1);
        if !self.send(Command::Pause { reply }) {
            return Err(SpeechError::Failed.into_ipc_error());
        }
        rx.recv()
            .unwrap_or(Err(SpeechError::Failed))
            .map_err(SpeechError::into_ipc_error)
    }

    pub fn resume(&self) -> Result<(), IpcError> {
        let (reply, rx) = mpsc::sync_channel(1);
        if !self.send(Command::Resume { reply }) {
            return Err(SpeechError::Failed.into_ipc_error());
        }
        rx.recv()
            .unwrap_or(Err(SpeechError::Failed))
            .map_err(SpeechError::into_ipc_error)
    }

    pub fn stop(&self) {
        let (reply, rx) = mpsc::sync_channel(1);
        if self.send(Command::Stop { reply }) {
            let _ = rx.recv();
        }
    }

    /// Stops playback and joins the speech worker thread.
    pub fn shutdown(&self) {
        if let Some(tx) = self
            .tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = tx.send(Command::Shutdown);
        }
        if let Some(handle) = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = handle.join();
        }
    }
}

impl Drop for SpeechService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_loop(backend: &mut dyn SpeechBackend, rx: mpsc::Receiver<Command>) {
    let generation = Arc::new(AtomicU64::new(0));
    let mut state = PlaybackState::Idle;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::ListVoices { reply } => {
                let _ = reply.send(backend.list_voices());
            }
            Command::Speak { args, sink, reply } => {
                // One utterance at a time: always stop the previous one and invalidate late events.
                let my_gen = generation.fetch_add(1, Ordering::SeqCst) + 1;
                if state != PlaybackState::Idle {
                    backend.stop();
                    state = PlaybackState::Idle;
                }

                let voices = match backend.list_voices() {
                    Ok(voices) => voices,
                    Err(err) => {
                        let _ = reply.send(Err(err));
                        continue;
                    }
                };
                if voices.is_empty() {
                    let _ = reply.send(Err(SpeechError::NoVoices));
                    continue;
                }
                if let Some(ref requested) = args.voice_id
                    && !voices.iter().any(|v| v.id == *requested)
                {
                    let _ = reply.send(Err(SpeechError::UnknownVoice));
                    continue;
                }

                let gen_ref = Arc::clone(&generation);
                let guarded_sink: EventSink = Arc::new(move |event| {
                    if gen_ref.load(Ordering::SeqCst) == my_gen {
                        sink(event);
                    }
                });

                let result = backend.speak(
                    &args.text,
                    args.voice_id.as_deref(),
                    args.rate,
                    guarded_sink,
                );
                if result.is_ok() {
                    state = PlaybackState::Playing;
                }
                let _ = reply.send(result);
            }
            Command::Pause { reply } => {
                let res = if state == PlaybackState::Playing {
                    let r = backend.pause();
                    if r.is_ok() {
                        state = PlaybackState::Paused;
                    }
                    r
                } else {
                    Ok(())
                };
                let _ = reply.send(res);
            }
            Command::Resume { reply } => {
                let res = if state == PlaybackState::Paused {
                    let r = backend.resume();
                    if r.is_ok() {
                        state = PlaybackState::Playing;
                    }
                    r
                } else {
                    Ok(())
                };
                let _ = reply.send(res);
            }
            Command::Stop { reply } => {
                generation.fetch_add(1, Ordering::SeqCst);
                backend.stop();
                state = PlaybackState::Idle;
                let _ = reply.send(());
            }
            Command::Shutdown => {
                generation.fetch_add(1, Ordering::SeqCst);
                backend.stop();
                break;
            }
        }
    }
}

/// RAII COM initializer for the dedicated speech worker thread.
struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn init() -> Self {
        // SAFETY: Called once on our dedicated speech thread before any WinRT calls; paired with
        // `CoUninitialize` in `Drop` when initialization succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self {
            initialized: hr.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: Matches the successful `CoInitializeEx` on this same thread.
            unsafe {
                CoUninitialize();
            }
        }
    }
}

/// Holds live WinRT playback objects for the current utterance so streams and event handlers
/// stay alive until stopped or replaced.
struct ActivePlayback {
    _stream: SpeechSynthesisStream,
    _source: MediaSource,
    _item: MediaPlaybackItem,
    player: MediaPlayer,
}

/// Production WinRT speech backend (`Windows.Media.SpeechSynthesis` + `Windows.Media.Playback`).
#[derive(Default)]
struct WinRtSpeechBackend {
    active: Option<ActivePlayback>,
}

/// Maps WinRT MediaPlayer / audio HRESULTs to whether the failure is due to a missing audio
/// output device (common on headless servers and CI runners).
fn classify_playback_error(hresult: i32) -> ErrorCode {
    const MF_E_NO_AUDIO_PLAYBACK_DEVICE: i32 = 0xC00D_36FA_u32 as i32;
    const MF_E_AUDIO_PLAYBACK_DEVICE_INVALIDATED: i32 = 0xC00D_4E86_u32 as i32;
    const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004_u32 as i32;
    const AUDCLNT_E_ENDPOINT_CREATE_FAILED: i32 = 0x8889_000F_u32 as i32;
    const AUDCLNT_E_SERVICE_NOT_RUNNING: i32 = 0x8889_0010_u32 as i32;
    const ERROR_NOT_FOUND_HR: i32 = 0x8007_0490_u32 as i32;

    match hresult {
        MF_E_NO_AUDIO_PLAYBACK_DEVICE
        | MF_E_AUDIO_PLAYBACK_DEVICE_INVALIDATED
        | AUDCLNT_E_DEVICE_INVALIDATED
        | AUDCLNT_E_ENDPOINT_CREATE_FAILED
        | AUDCLNT_E_SERVICE_NOT_RUNNING
        | ERROR_NOT_FOUND_HR => ErrorCode::SpeechNoAudioDevice,
        _ => ErrorCode::SpeechFailed,
    }
}

fn attach_word_boundary_tracks(
    tracks: &MediaPlaybackTimedMetadataTrackList,
    registered: &Arc<Mutex<HashSet<u32>>>,
    sink: &EventSink,
) {
    let Ok(count) = tracks.Size() else {
        return;
    };
    for i in 0..count {
        let Ok(track) = tracks.GetAt(i) else {
            continue;
        };
        let Ok(label) = track.Label() else {
            continue;
        };
        if label != "SpeechWord" {
            continue;
        }
        let newly_registered = registered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(i);
        if !newly_registered {
            continue;
        }
        let word_sink = Arc::clone(sink);
        let _ = track.CueEntered(
            &TypedEventHandler::<TimedMetadataTrack, MediaCueEventArgs>::new(
                move |_track, args| {
                    if let Some(args) = args.as_ref()
                        && let Ok(cue) = args.Cue()
                        && let Ok(speech_cue) = cue.cast::<SpeechCue>()
                        && let Ok(start_ref) = speech_cue.StartPositionInInput()
                        && let Ok(end_ref) = speech_cue.EndPositionInInput()
                        && let Ok(start) = start_ref.Value()
                        && let Ok(end) = end_ref.Value()
                        && start >= 0
                        && end >= start
                    {
                        let utf16_start = start as u32;
                        let utf16_len = (end - start + 1) as u32;
                        word_sink(SpeechEvent::Word {
                            utf16_start,
                            utf16_len,
                        });
                    }
                    Ok(())
                },
            ),
        );
        let _ =
            tracks.SetPresentationMode(i, TimedMetadataTrackPresentationMode::ApplicationPresented);
    }
}

impl SpeechBackend for WinRtSpeechBackend {
    fn list_voices(&mut self) -> Result<Vec<SpeechVoice>, SpeechError> {
        let default_id = SpeechSynthesizer::DefaultVoice()
            .ok()
            .and_then(|v| v.Id().ok())
            .map(|id| id.to_string());
        let all = SpeechSynthesizer::AllVoices().map_err(|_| SpeechError::Failed)?;
        let count = all
            .Size()
            .map_err(|_| SpeechError::Failed)?
            .min(MAX_SPEECH_VOICES);
        let mut voices = Vec::with_capacity(count as usize);
        for i in 0..count {
            let Ok(v) = all.GetAt(i) else {
                continue;
            };
            let (Ok(id), Ok(name), Ok(language)) = (v.Id(), v.DisplayName(), v.Language()) else {
                continue;
            };
            let id_str = id.to_string();
            let is_default = default_id.as_deref() == Some(id_str.as_str());
            voices.push(SpeechVoice {
                id: id_str,
                name: name.to_string(),
                language: language.to_string(),
                is_default,
            });
        }
        Ok(voices)
    }

    fn speak(
        &mut self,
        text: &str,
        voice_id: Option<&str>,
        rate: f64,
        sink: EventSink,
    ) -> Result<(), SpeechError> {
        self.stop();

        let all = SpeechSynthesizer::AllVoices().map_err(|_| SpeechError::Failed)?;
        let count = all.Size().map_err(|_| SpeechError::Failed)?;
        if count == 0 {
            return Err(SpeechError::NoVoices);
        }

        // Security rule: voice_id is only ever matched against `AllVoices()` and never passed
        // to filesystem or registry APIs.
        let selected_voice = if let Some(requested) = voice_id {
            let mut found = None;
            for i in 0..count {
                if let Ok(v) = all.GetAt(i)
                    && let Ok(id) = v.Id()
                    && id == requested
                {
                    found = Some(v);
                    break;
                }
            }
            Some(found.ok_or(SpeechError::UnknownVoice)?)
        } else {
            None
        };

        let synth = SpeechSynthesizer::new().map_err(|_| SpeechError::Failed)?;
        if let Some(voice) = selected_voice {
            synth.SetVoice(&voice).map_err(|_| SpeechError::Failed)?;
        }
        let opts = synth.Options().map_err(|_| SpeechError::Failed)?;
        opts.SetIncludeWordBoundaryMetadata(true)
            .map_err(|_| SpeechError::Failed)?;
        opts.SetSpeakingRate(rate)
            .map_err(|_| SpeechError::Failed)?;

        // Security rule: synthesize plain text ONLY (never `SynthesizeSsmlToStreamAsync`).
        let stream = synth
            .SynthesizeTextToStreamAsync(&HSTRING::from(text))
            .map_err(|_| SpeechError::Failed)?
            .join()
            .map_err(|_| SpeechError::Failed)?;

        // Security rule: playback comes ONLY from the in-memory stream (never `CreateFromUri`).
        let content_type = stream.ContentType().map_err(|_| SpeechError::Failed)?;
        let source = MediaSource::CreateFromStream(&stream, &content_type)
            .map_err(|_| SpeechError::Failed)?;
        let item = MediaPlaybackItem::Create(&source).map_err(|_| SpeechError::Failed)?;

        let registered = Arc::new(Mutex::new(HashSet::new()));
        if let Ok(tracks) = item.TimedMetadataTracks() {
            attach_word_boundary_tracks(&tracks, &registered, &sink);
        }
        let tracks_sink = Arc::clone(&sink);
        let tracks_registered = Arc::clone(&registered);
        let _ = item.TimedMetadataTracksChanged(&TypedEventHandler::<
            MediaPlaybackItem,
            IVectorChangedEventArgs,
        >::new(move |sender, _args| {
            if let Some(item) = sender.as_ref()
                && let Ok(tracks) = item.TimedMetadataTracks()
            {
                attach_word_boundary_tracks(&tracks, &tracks_registered, &tracks_sink);
            }
            Ok(())
        }));

        let player = MediaPlayer::new().map_err(|_| SpeechError::Failed)?;
        let ended_sink = Arc::clone(&sink);
        let _ = player.MediaEnded(
            &TypedEventHandler::<MediaPlayer, windows::core::IInspectable>::new(
                move |_player, _args| {
                    ended_sink(SpeechEvent::Finished);
                    Ok(())
                },
            ),
        );
        let failed_sink = Arc::clone(&sink);
        let _ = player.MediaFailed(
            &TypedEventHandler::<MediaPlayer, MediaPlayerFailedEventArgs>::new(
                move |_player, args| {
                    let hr = args
                        .as_ref()
                        .and_then(|a| a.ExtendedErrorCode().ok())
                        .map_or(0, |h| h.0);
                    let code = classify_playback_error(hr);
                    failed_sink(SpeechEvent::Error { code });
                    Ok(())
                },
            ),
        );

        player.SetSource(&item).map_err(|e| {
            if classify_playback_error(e.code().0) == ErrorCode::SpeechNoAudioDevice {
                SpeechError::NoAudioDevice
            } else {
                SpeechError::Failed
            }
        })?;
        player.Play().map_err(|e| {
            if classify_playback_error(e.code().0) == ErrorCode::SpeechNoAudioDevice {
                SpeechError::NoAudioDevice
            } else {
                SpeechError::Failed
            }
        })?;

        self.active = Some(ActivePlayback {
            _stream: stream,
            _source: source,
            _item: item,
            player,
        });
        Ok(())
    }

    fn pause(&mut self) -> Result<(), SpeechError> {
        if let Some(active) = &self.active {
            active.player.Pause().map_err(|_| SpeechError::Failed)?;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), SpeechError> {
        if let Some(active) = &self.active {
            active.player.Play().map_err(|_| SpeechError::Failed)?;
        }
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(active) = self.active.take() {
            let _ = active.player.Pause();
            let _ = active.player.Close();
        }
    }
}

impl Drop for WinRtSpeechBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc_contract::limits::MAX_SPEECH_TEXT_BYTES;

    #[derive(Default)]
    struct FakeState {
        voices: Vec<SpeechVoice>,
        speak_calls: Vec<(String, Option<String>, f64)>,
        sinks: Vec<EventSink>,
        pauses: u32,
        resumes: u32,
        stops: u32,
        fail_speak: Option<SpeechError>,
    }

    struct FakeBackend {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeBackend {
        fn with_voices(voices: Vec<SpeechVoice>) -> (Self, Arc<Mutex<FakeState>>) {
            let state = Arc::new(Mutex::new(FakeState {
                voices,
                ..FakeState::default()
            }));
            (
                Self {
                    state: Arc::clone(&state),
                },
                state,
            )
        }
    }

    impl SpeechBackend for FakeBackend {
        fn list_voices(&mut self) -> Result<Vec<SpeechVoice>, SpeechError> {
            Ok(self.state.lock().unwrap().voices.clone())
        }

        fn speak(
            &mut self,
            text: &str,
            voice_id: Option<&str>,
            rate: f64,
            sink: EventSink,
        ) -> Result<(), SpeechError> {
            let mut st = self.state.lock().unwrap();
            if let Some(err) = st.fail_speak {
                return Err(err);
            }
            st.speak_calls
                .push((text.to_owned(), voice_id.map(str::to_owned), rate));
            st.sinks.push(sink);
            Ok(())
        }

        fn pause(&mut self) -> Result<(), SpeechError> {
            self.state.lock().unwrap().pauses += 1;
            Ok(())
        }

        fn resume(&mut self) -> Result<(), SpeechError> {
            self.state.lock().unwrap().resumes += 1;
            Ok(())
        }

        fn stop(&mut self) {
            self.state.lock().unwrap().stops += 1;
        }
    }

    fn sample_voices() -> Vec<SpeechVoice> {
        vec![
            SpeechVoice {
                id: "voice-zh".into(),
                name: "Microsoft Hanhan".into(),
                language: "zh-TW".into(),
                is_default: true,
            },
            SpeechVoice {
                id: "voice-en".into(),
                name: "Microsoft David".into(),
                language: "en-US".into(),
                is_default: false,
            },
        ]
    }

    #[test]
    fn one_utterance_at_a_time_stops_previous_and_drops_stale_events() {
        let (backend, state) = FakeBackend::with_voices(sample_voices());
        let service = SpeechService::with_backend(backend);

        let events_1 = Arc::new(Mutex::new(Vec::new()));
        let e1 = Arc::clone(&events_1);
        service
            .speak(
                SpeakArgs {
                    text: "first".into(),
                    voice_id: None,
                    rate: 1.0,
                },
                Arc::new(move |ev| e1.lock().unwrap().push(ev)),
            )
            .unwrap();

        let events_2 = Arc::new(Mutex::new(Vec::new()));
        let e2 = Arc::clone(&events_2);
        service
            .speak(
                SpeakArgs {
                    text: "second".into(),
                    voice_id: Some("voice-en".into()),
                    rate: 1.5,
                },
                Arc::new(move |ev| e2.lock().unwrap().push(ev)),
            )
            .unwrap();

        let st = state.lock().unwrap();
        assert_eq!(st.stops, 1, "starting second utterance must stop first");
        assert_eq!(st.speak_calls.len(), 2);
        assert_eq!(st.speak_calls[1].1.as_deref(), Some("voice-en"));
        assert_eq!(st.speak_calls[1].2, 1.5);

        // Stale event emitted on sink 0 must be dropped; event on sink 1 must arrive.
        (st.sinks[0])(SpeechEvent::Word {
            utf16_start: 0,
            utf16_len: 5,
        });
        (st.sinks[1])(SpeechEvent::Word {
            utf16_start: 0,
            utf16_len: 6,
        });
        (st.sinks[1])(SpeechEvent::Finished);
        drop(st);

        assert!(events_1.lock().unwrap().is_empty());
        assert_eq!(
            *events_2.lock().unwrap(),
            vec![
                SpeechEvent::Word {
                    utf16_start: 0,
                    utf16_len: 6
                },
                SpeechEvent::Finished
            ]
        );
    }

    #[test]
    fn pause_resume_stop_state_transitions() {
        let (backend, state) = FakeBackend::with_voices(sample_voices());
        let service = SpeechService::with_backend(backend);

        // Pause / resume while idle are no-ops.
        service.pause().unwrap();
        service.resume().unwrap();
        assert_eq!(state.lock().unwrap().pauses, 0);
        assert_eq!(state.lock().unwrap().resumes, 0);

        service
            .speak(
                SpeakArgs {
                    text: "hello".into(),
                    voice_id: None,
                    rate: 1.0,
                },
                Arc::new(|_| {}),
            )
            .unwrap();

        service.pause().unwrap();
        service.pause().unwrap(); // second pause while already paused is a no-op
        assert_eq!(state.lock().unwrap().pauses, 1);

        service.resume().unwrap();
        service.resume().unwrap(); // second resume while already playing is a no-op
        assert_eq!(state.lock().unwrap().resumes, 1);

        service.stop();
        assert_eq!(state.lock().unwrap().stops, 1);

        // After stop, pause and resume are no-ops again.
        service.pause().unwrap();
        service.resume().unwrap();
        assert_eq!(state.lock().unwrap().pauses, 1);
        assert_eq!(state.lock().unwrap().resumes, 1);
    }

    #[test]
    fn rejects_oversized_text_out_of_range_rate_unknown_voice_and_never_leaks_text() {
        let (backend, state) = FakeBackend::with_voices(sample_voices());
        let service = SpeechService::with_backend(backend);

        let secret = "CONFIDENTIAL_DOC_BODY_599";

        // 1. Oversized text
        let err = service
            .speak(
                SpeakArgs {
                    text: format!("{secret}{}", "x".repeat(MAX_SPEECH_TEXT_BYTES as usize)),
                    voice_id: None,
                    rate: 1.0,
                },
                Arc::new(|_| {}),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(!err.message.contains(secret));

        // 2. Rate out of range
        for bad_rate in [0.4, 6.1, f64::NAN, f64::INFINITY] {
            let err = service
                .speak(
                    SpeakArgs {
                        text: secret.into(),
                        voice_id: None,
                        rate: bad_rate,
                    },
                    Arc::new(|_| {}),
                )
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument);
            assert!(!err.message.contains(secret));
        }

        // 3. Unknown voice id (not in listed voices)
        let err = service
            .speak(
                SpeakArgs {
                    text: secret.into(),
                    voice_id: Some("not-installed-voice".into()),
                    rate: 1.0,
                },
                Arc::new(|_| {}),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SpeechUnknownVoice);
        assert!(!err.message.contains(secret));
        assert!(!err.message.contains("not-installed-voice"));

        // 4. Engine failure / no audio device
        state.lock().unwrap().fail_speak = Some(SpeechError::NoAudioDevice);
        let err = service
            .speak(
                SpeakArgs {
                    text: secret.into(),
                    voice_id: None,
                    rate: 1.0,
                },
                Arc::new(|_| {}),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SpeechNoAudioDevice);
        assert!(!err.message.contains(secret));
    }

    #[test]
    fn no_voices_returns_empty_list_and_explicit_speak_error() {
        let (backend, _state) = FakeBackend::with_voices(Vec::new());
        let service = SpeechService::with_backend(backend);

        assert!(service.list_voices().unwrap().is_empty());
        let err = service
            .speak(
                SpeakArgs {
                    text: "secret text".into(),
                    voice_id: None,
                    rate: 1.0,
                },
                Arc::new(|_| {}),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SpeechNoVoices);
        assert!(!err.message.contains("secret text"));
    }

    #[test]
    fn dropping_service_stops_active_playback() {
        let (backend, state) = FakeBackend::with_voices(sample_voices());
        {
            let service = SpeechService::with_backend(backend);
            service
                .speak(
                    SpeakArgs {
                        text: "reading".into(),
                        voice_id: None,
                        rate: 1.0,
                    },
                    Arc::new(|_| {}),
                )
                .unwrap();
        }
        assert!(
            state.lock().unwrap().stops >= 1,
            "dropping SpeechService must stop playback on the worker thread"
        );
    }

    #[test]
    fn speak_args_debug_omits_spoken_text() {
        let args = SpeakArgs {
            text: "SENSITIVE_CONTENT_XYZ".into(),
            voice_id: Some("v1".into()),
            rate: 1.0,
        };
        let dbg = format!("{args:?}");
        assert!(!dbg.contains("SENSITIVE_CONTENT_XYZ"));
        assert!(dbg.contains("text_bytes"));
    }
}

/// Real Windows WinRT synthesis-only tests (in-memory only; never plays audio).
#[cfg(all(test, windows))]
mod winrt_tests {
    use super::*;

    #[test]
    fn winrt_speech_synthesizes_in_memory_with_utf16_word_boundaries() {
        let _com = ComGuard::init();
        let voices = SpeechSynthesizer::AllVoices().expect("AllVoices should succeed");
        let count = voices.Size().expect("voices.Size");

        if count == 0 {
            let mut backend = WinRtSpeechBackend::default();
            let err = backend
                .speak("hello", None, 1.0, Arc::new(|_| {}))
                .expect_err("speak without voices must fail with NoVoices");
            assert_eq!(err, SpeechError::NoVoices);
            return;
        }

        let synth = SpeechSynthesizer::new().expect("SpeechSynthesizer::new");
        let opts = synth.Options().expect("Options");
        opts.SetIncludeWordBoundaryMetadata(true)
            .expect("SetIncludeWordBoundaryMetadata");

        // Mixed Chinese + English + non-BMP U+20000 (surrogate pair = 2 UTF-16 code units).
        let input = "你好 world \u{20000} test";
        let utf16: Vec<u16> = input.encode_utf16().collect();
        let stream = synth
            .SynthesizeTextToStreamAsync(&HSTRING::from(input))
            .expect("SynthesizeTextToStreamAsync")
            .join()
            .expect("join synthesis");

        assert!(
            stream.Size().expect("stream.Size") > 0,
            "synthesized stream must contain audio bytes"
        );

        let tracks = stream.TimedMetadataTracks().expect("TimedMetadataTracks");
        let mut cues_found = Vec::new();
        for t_idx in 0..tracks.Size().expect("tracks.Size") {
            let track = tracks.GetAt(t_idx).unwrap();
            if track.Label().unwrap() == "SpeechWord" {
                let cues = track.Cues().unwrap();
                for c_idx in 0..cues.Size().unwrap() {
                    let speech_cue: SpeechCue = cues.GetAt(c_idx).unwrap().cast().unwrap();
                    let start = speech_cue.StartPositionInInput().unwrap().Value().unwrap();
                    let end = speech_cue.EndPositionInInput().unwrap().Value().unwrap();
                    let text = speech_cue.Text().unwrap().to_string();
                    assert!(start >= 0 && end >= start);
                    let slice = String::from_utf16(&utf16[(start as usize)..=(end as usize)])
                        .expect("cue range must be a valid UTF-16 slice");
                    assert_eq!(
                        slice, text,
                        "SpeechCue positions must index input in UTF-16 code units"
                    );
                    cues_found.push((start as u32, (end - start + 1) as u32, text));
                }
            }
        }
        assert!(
            !cues_found.is_empty(),
            "expected at least one SpeechWord cue"
        );
        // Verify that "test" after the non-BMP surrogate pair sits at UTF-16 index 12 (not scalar 11).
        let test_cue = cues_found
            .iter()
            .find(|(_, _, text)| text == "test")
            .expect("expected cue for 'test'");
        assert_eq!(
            test_cue.0, 12,
            "word after non-BMP char must use UTF-16 index"
        );
        assert_eq!(test_cue.1, 4);
    }

    #[test]
    fn winrt_speech_treats_ssml_and_sapi_markup_as_plain_text() {
        let _com = ComGuard::init();
        let voices = SpeechSynthesizer::AllVoices().expect("AllVoices should succeed");
        let count = voices.Size().expect("voices.Size");

        if count == 0 {
            let mut backend = WinRtSpeechBackend::default();
            let err = backend
                .speak(
                    r#"<speak><audio src="http://example.invalid/a.wav"/></speak>"#,
                    None,
                    1.0,
                    Arc::new(|_| {}),
                )
                .expect_err("speak without voices must fail with NoVoices");
            assert_eq!(err, SpeechError::NoVoices);
            return;
        }

        let synth = SpeechSynthesizer::new().expect("SpeechSynthesizer::new");
        synth
            .Options()
            .unwrap()
            .SetIncludeWordBoundaryMetadata(true)
            .unwrap();

        let ssml = r#"<speak><audio src="http://example.invalid/a.wav"/></speak>"#;
        let sapi = r#"<voice required="Name=x"/>"#;

        for sample in [ssml, sapi] {
            let stream = synth
                .SynthesizeTextToStreamAsync(&HSTRING::from(sample))
                .expect("SynthesizeTextToStreamAsync")
                .join()
                .expect("join synthesis");
            let tracks = stream.TimedMetadataTracks().unwrap();
            let mut words = Vec::new();
            for t_idx in 0..tracks.Size().unwrap() {
                let track = tracks.GetAt(t_idx).unwrap();
                if track.Label().unwrap() == "SpeechWord" {
                    let cues = track.Cues().unwrap();
                    for c_idx in 0..cues.Size().unwrap() {
                        let cue: SpeechCue = cues.GetAt(c_idx).unwrap().cast().unwrap();
                        words.push(cue.Text().unwrap().to_string().to_lowercase());
                    }
                }
            }
            if sample == ssml {
                assert!(
                    words.iter().any(|w| w.contains("audio")),
                    "SSML <audio> must be spoken as plain text; cues={words:?}"
                );
                assert!(
                    words.iter().any(|w| w.contains("src")),
                    "SSML src attribute must be spoken as plain text; cues={words:?}"
                );
            } else {
                assert!(
                    words.iter().any(|w| w.contains("voice")),
                    "SAPI <voice> must be spoken as plain text; cues={words:?}"
                );
            }
        }
    }
}

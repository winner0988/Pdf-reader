// Read-aloud IPC commands (B6-14, #599, docs/architecture/read-aloud.md).
// Plain text is sent to the main process, which synthesizes audio in memory with installed
// offline Windows voices and streams UTF-16 word boundary events back on a channel.

import { Channel, invoke } from "@tauri-apps/api/core";

import type { SpeakArgs, SpeechEvent, SpeechVoice } from "@/ipc/generated/contract";

export type SpeechApi = {
  getVoices(): Promise<SpeechVoice[]>;
  speak(args: SpeakArgs, onEvent: (event: SpeechEvent) => void): Promise<void>;
  pause(): Promise<void>;
  resume(): Promise<void>;
  stop(): Promise<void>;
};

export const tauriSpeechApi: SpeechApi = {
  getVoices: () => invoke<SpeechVoice[]>("get_speech_voices"),
  speak: (args, onEvent) => {
    const channel = new Channel<SpeechEvent>();
    channel.onmessage = onEvent;
    return invoke<void>("speak_text", { args, onEvent: channel });
  },
  pause: () => invoke<void>("pause_speech"),
  resume: () => invoke<void>("resume_speech"),
  stop: () => invoke<void>("stop_speech"),
};

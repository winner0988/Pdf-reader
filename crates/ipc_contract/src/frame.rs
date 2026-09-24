//! Length-prefixed framing for the main <-> worker pipe.
//!
//! A frame is a little-endian `u32` payload length followed by the payload, a postcard-encoded
//! [`crate::worker::WorkerRequest`] or [`crate::worker::WorkerResponse`]. The length is checked
//! against [`MAX_FRAME_BYTES`] before any buffer is allocated, and decoding rejects trailing
//! bytes, so a malicious worker can neither force a huge allocation nor smuggle extra data.

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use zeroize::Zeroize;

use crate::PROTOCOL_VERSION;
use crate::limits::MAX_FRAME_BYTES;
use crate::worker::WorkerResponse;

pub const FRAME_HEADER_BYTES: usize = 4;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame of {len} bytes exceeds the {max} byte limit")]
    TooLarge { len: usize, max: usize },
    #[error("stream ended in the middle of a frame")]
    Truncated,
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("failed to encode message: {0}")]
    Encode(postcard::Error),
    #[error("failed to decode message: {0}")]
    Decode(postcard::Error),
    #[error("{0} unexpected bytes after the message")]
    TrailingBytes(usize),
    #[error("first message from the worker was not Hello")]
    MissingHello,
    #[error("protocol version mismatch: expected {expected}, worker speaks {actual}")]
    VersionMismatch { expected: u32, actual: u32 },
}

/// Encodes a message into a frame payload (without the length prefix).
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, FrameError> {
    let payload = postcard::to_stdvec(message).map_err(FrameError::Encode)?;
    check_len(payload.len(), MAX_FRAME_BYTES)?;
    Ok(payload)
}

/// Decodes a frame payload, rejecting trailing bytes.
pub fn decode<T: DeserializeOwned>(payload: &[u8]) -> Result<T, FrameError> {
    let (message, rest) = postcard::take_from_bytes(payload).map_err(FrameError::Decode)?;
    if !rest.is_empty() {
        return Err(FrameError::TrailingBytes(rest.len()));
    }
    Ok(message)
}

/// Writes one frame.
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), FrameError> {
    write_frame_with_limit(writer, payload, MAX_FRAME_BYTES)
}

/// Reads one frame. Returns `Ok(None)` on a clean end of stream between frames.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, FrameError> {
    read_frame_with_limit(reader, MAX_FRAME_BYTES)
}

/// Encodes and writes one message.
pub fn send<W: Write, T: Serialize>(writer: &mut W, message: &T) -> Result<(), FrameError> {
    write_frame(writer, &encode(message)?)
}

/// Reads and decodes one message. Returns `Ok(None)` on a clean end of stream.
pub fn receive<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<Option<T>, FrameError> {
    read_frame(reader)?
        .map(|payload| decode(&payload))
        .transpose()
}

/// Like [`send`], then wipes the encoded bytes. For requests, which may carry a password
/// (MVP-16); they are small, so wiping costs nothing measurable.
pub fn send_wiped<W: Write, T: Serialize>(writer: &mut W, message: &T) -> Result<(), FrameError> {
    let mut payload = encode(message)?;
    let written = write_frame(writer, &payload);
    payload.zeroize();
    written
}

/// Like [`receive`], then wipes the received bytes: the decoded message has its own copy (a
/// password in it wipes itself when dropped).
pub fn receive_wiped<R: Read, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<Option<T>, FrameError> {
    let Some(mut payload) = read_frame(reader)? else {
        return Ok(None);
    };
    let message = decode(&payload);
    payload.zeroize();
    message.map(Some)
}

/// Checks the worker's first message: it must be `Hello` with our protocol version.
pub fn check_hello(first: &WorkerResponse) -> Result<(), FrameError> {
    match first {
        WorkerResponse::Hello {
            protocol_version, ..
        } if *protocol_version == PROTOCOL_VERSION => Ok(()),
        WorkerResponse::Hello {
            protocol_version, ..
        } => Err(FrameError::VersionMismatch {
            expected: PROTOCOL_VERSION,
            actual: *protocol_version,
        }),
        _ => Err(FrameError::MissingHello),
    }
}

fn check_len(len: usize, max: usize) -> Result<(), FrameError> {
    if len > max {
        return Err(FrameError::TooLarge { len, max });
    }
    Ok(())
}

fn write_frame_with_limit<W: Write>(
    writer: &mut W,
    payload: &[u8],
    max: usize,
) -> Result<(), FrameError> {
    check_len(payload.len(), max)?;
    let len = u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge {
        len: payload.len(),
        max,
    })?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

fn read_frame_with_limit<R: Read>(
    reader: &mut R,
    max: usize,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut header = [0u8; FRAME_HEADER_BYTES];
    let mut filled = 0;
    while filled < header.len() {
        match reader.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => filled += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err.into()),
        }
    }

    let len = u32::from_le_bytes(header) as usize;
    check_len(len, max)?;

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).map_err(|err| {
        if err.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            err.into()
        }
    })?;
    Ok(Some(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DocumentId, RequestId, Rotation};
    use crate::worker::WorkerRequest;

    fn render_request() -> WorkerRequest {
        WorkerRequest::Render {
            request: RequestId(7),
            doc: DocumentId(1),
            page_index: 3,
            scale: 1.5,
            rotation: Rotation::Cw90,
        }
    }

    #[test]
    fn message_round_trips_through_a_stream() {
        let mut stream = Vec::new();
        send(&mut stream, &render_request()).unwrap();
        send(&mut stream, &WorkerRequest::Shutdown).unwrap();

        let mut reader = stream.as_slice();
        let first: WorkerRequest = receive(&mut reader).unwrap().unwrap();
        let second: WorkerRequest = receive(&mut reader).unwrap().unwrap();
        let end: Option<WorkerRequest> = receive(&mut reader).unwrap();

        assert_eq!(first, render_request());
        assert_eq!(second, WorkerRequest::Shutdown);
        assert_eq!(end, None);
    }

    #[test]
    fn wiped_frames_carry_the_same_messages() {
        let request = WorkerRequest::Open {
            request: RequestId(3),
            doc: DocumentId(1),
            file: crate::worker::FileHandle(8),
            password: Some(crate::types::Password::new("secret".to_owned())),
        };
        let mut wire = Vec::new();
        send_wiped(&mut wire, &request).unwrap();
        let mut plain = Vec::new();
        send(&mut plain, &request).unwrap();
        assert_eq!(wire, plain);
        let received: Option<WorkerRequest> = receive_wiped(&mut wire.as_slice()).unwrap();
        assert_eq!(received, Some(request));
        assert!(
            receive_wiped::<_, WorkerRequest>(&mut [].as_slice())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn oversized_length_is_rejected_before_allocating() {
        // Header claims 1 GiB; no payload follows. Must fail on the header alone.
        let stream = (1u32 << 30).to_le_bytes();
        let err = read_frame(&mut stream.as_slice()).unwrap_err();
        assert!(matches!(err, FrameError::TooLarge { len, .. } if len == 1 << 30));
    }

    #[test]
    fn oversized_payload_is_not_written() {
        let mut sink = Vec::new();
        let err = write_frame_with_limit(&mut sink, &[0u8; 17], 16).unwrap_err();
        assert!(matches!(err, FrameError::TooLarge { len: 17, max: 16 }));
        assert!(sink.is_empty());
    }

    #[test]
    fn frame_at_the_limit_is_accepted() {
        let mut stream = Vec::new();
        write_frame_with_limit(&mut stream, &[1u8; 16], 16).unwrap();
        let payload = read_frame_with_limit(&mut stream.as_slice(), 16)
            .unwrap()
            .unwrap();
        assert_eq!(payload, vec![1u8; 16]);
    }

    #[test]
    fn truncated_header_and_payload_are_errors() {
        let err = read_frame(&mut [5u8, 0].as_slice()).unwrap_err();
        assert!(matches!(err, FrameError::Truncated));

        let mut stream = 10u32.to_le_bytes().to_vec();
        stream.extend_from_slice(&[0u8; 3]);
        let err = read_frame(&mut stream.as_slice()).unwrap_err();
        assert!(matches!(err, FrameError::Truncated));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut payload = encode(&WorkerRequest::Shutdown).unwrap();
        payload.push(0);
        let err = decode::<WorkerRequest>(&payload).unwrap_err();
        assert!(matches!(err, FrameError::TrailingBytes(1)));
    }

    #[test]
    fn unknown_variant_is_rejected() {
        let err = decode::<WorkerRequest>(&[200]).unwrap_err();
        assert!(matches!(err, FrameError::Decode(_)));
    }

    #[test]
    fn hello_is_variant_zero() {
        let payload = encode(&WorkerResponse::hello()).unwrap();
        assert_eq!(
            payload[0], 0,
            "Hello must stay the first WorkerResponse variant"
        );
    }

    #[test]
    fn matching_hello_is_accepted() {
        check_hello(&WorkerResponse::hello()).unwrap();
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let hello = WorkerResponse::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
            worker_version: "9.9.9".to_owned(),
        };
        let err = check_hello(&hello).unwrap_err();
        assert!(matches!(
            err,
            FrameError::VersionMismatch { expected, actual }
                if expected == PROTOCOL_VERSION && actual == PROTOCOL_VERSION + 1
        ));
    }

    #[test]
    fn non_hello_first_message_is_rejected() {
        let first = WorkerResponse::PageSearched {
            request: RequestId(1),
            page_index: 0,
            hits: Vec::new(),
            has_text: false,
        };
        assert!(matches!(
            check_hello(&first).unwrap_err(),
            FrameError::MissingHello
        ));
    }

    #[test]
    fn hello_from_a_newer_worker_still_decodes() {
        // A future worker with a different version must still produce a decodable Hello,
        // so the main process can report the mismatch instead of a generic decode error.
        let payload = encode(&WorkerResponse::Hello {
            protocol_version: 42,
            worker_version: "2.0.0".to_owned(),
        })
        .unwrap();
        let decoded: WorkerResponse = decode(&payload).unwrap();
        assert!(matches!(
            check_hello(&decoded),
            Err(FrameError::VersionMismatch { actual: 42, .. })
        ));
    }
}

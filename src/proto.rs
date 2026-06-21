//! Control-protocol message types and wire framing.
//!
//! Mirrors `docs/protocol.md`. This is the binary↔binary control channel only;
//! it is independent of the per-mode data paths.
//!
//! Framing (§2): each message is a 4-byte big-endian (network order) `u32`
//! length prefix followed by that many bytes of UTF-8 JSON. The payload cap is
//! andri's own self-imposed limit, not an RFC requirement.

use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Protocol version. Bumped only on incompatible wire changes (§7).
pub const PROTOCOL_VERSION: u16 = 1;

/// Maximum control-message payload size (§2). Self-imposed cap, not a standard.
pub const MAX_FRAME_BYTES: u32 = 64 * 1024;

/// A control-channel message. Tagged so the wire carries a discriminant (§3).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum Msg {
    Hello(Hello),
    Welcome(Welcome),
    Negotiate(Negotiate),
    Start(Start),
    Run,
    Stop,
    Result(TestResult),
    Error(ProtoError),
}

/// §3.1 — client → server greeting.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Hello {
    pub protocol_version: u16,
    pub client_version: String,
    pub nonce: u64,
}

/// §3.2 — server → client greeting reply.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Welcome {
    pub protocol_version: u16,
    pub server_version: String,
    pub nonce: u64,
    pub accepted: bool,
}

/// Measurement mode (§3.3).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Tcp,
    Udp,
    File,
}

/// §3.3 — client → server test parameters.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Negotiate {
    pub mode: Mode,
    pub duration_secs: u64,
    pub warmup_secs: u64,
    pub parallel: u32,
    pub buffer_bytes: usize,
    pub bidir: bool,
    // UDP only:
    pub bitrate_bps: Option<u64>,
    pub packet_bytes: Option<usize>,
    // File only:
    pub file_len: Option<u64>,
    pub null_source: Option<bool>,
}

/// §3.4 — server → client; data listener is ready.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Start {
    pub data_port: u16,
    pub server_seed: u64,
}

/// Direction a result describes (§3.6). Both are sent for `bidir`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RoleDir {
    Send,
    Receive,
}

/// §3.6 — server → client final results.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestResult {
    pub role: RoleDir,
    pub bytes: u64,
    pub duration_secs: f64,
    pub bits_per_sec: f64,
    pub bytes_per_sec: f64,
    // UDP only:
    pub packets_expected: Option<u64>,
    pub packets_received: Option<u64>,
    pub packets_lost: Option<u64>,
    pub loss_ratio: Option<f64>,
    pub jitter_ms: Option<f64>,
    /// Per-second time series (§3.6); may be empty.
    pub samples: Vec<Sample>,
}

/// One once-per-second reading (§3.6).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Sample {
    pub t_secs: f64,
    pub bytes: u64,
    pub bits_per_sec: f64,
    // UDP only:
    pub packets_lost: Option<u64>,
    pub jitter_ms: Option<f64>,
}

/// §3.7 — terminal error. The sender closes the connection after sending it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtoError {
    VersionMismatch,
    InvalidParams,
    UnexpectedMessage,
    DataConnectFailed,
    Timeout,
    Internal,
}

/// Write one framed message: 4-byte BE length prefix + JSON payload (§2).
pub async fn write_msg<W>(w: &mut W, msg: &Msg) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let payload = serde_json::to_vec(msg)?;
    if payload.len() as u64 > MAX_FRAME_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control message exceeds MAX_FRAME_BYTES",
        ));
    }
    w.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    w.write_all(&payload).await?;
    w.flush().await?;
    Ok(())
}

/// Read one framed message. Rejects an oversized declared length per §2.
pub async fn read_msg<R>(r: &mut R) -> io::Result<Msg>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "declared frame length exceeds MAX_FRAME_BYTES",
        ));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).await?;
    let msg = serde_json::from_slice(&payload)?;
    Ok(msg)
}

/// Unit tests for the control-protocol framing and message types.
///
/// These run under `cargo test` only (`#[cfg(test)]`), so they don't affect the
/// shipped binary. They use an in-memory `Cursor<Vec<u8>>` as a fake stream —
/// `write_msg`/`read_msg` are generic over `AsyncWrite`/`AsyncRead`, so no real
/// socket is needed. See `docs/testing.md` for the overall test strategy.
///
/// Coverage:
/// - `roundtrip_all_variants` — every `Msg` survives write→read unchanged.
/// - `frame_has_be_length_prefix` — the on-wire frame matches `docs/protocol.md` §2.
/// - `oversized_declared_length_is_rejected` — the `MAX_FRAME_BYTES` guard fires.
/// - `enum_wire_tokens` — enums serialize to the lowercase tokens the spec shows.
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A representative `Negotiate` covering the optional UDP/file fields.
    fn sample_negotiate() -> Msg {
        Msg::Negotiate(Negotiate {
            mode: Mode::Udp,
            duration_secs: 10,
            warmup_secs: 1,
            parallel: 4,
            buffer_bytes: 65536,
            bidir: true,
            bitrate_bps: Some(1_000_000_000),
            packet_bytes: Some(1472),
            file_len: None,
            null_source: None,
        })
    }

    fn sample_result() -> Msg {
        Msg::Result(TestResult {
            role: RoleDir::Receive,
            bytes: 1234,
            duration_secs: 10.0,
            bits_per_sec: 987.6,
            bytes_per_sec: 123.4,
            packets_expected: Some(100),
            packets_received: Some(98),
            packets_lost: Some(2),
            loss_ratio: Some(0.02),
            jitter_ms: Some(0.5),
            samples: vec![Sample {
                t_secs: 1.0,
                bytes: 100,
                bits_per_sec: 800.0,
                packets_lost: Some(1),
                jitter_ms: Some(0.4),
            }],
        })
    }

    /// write_msg → read_msg must reproduce the message for each variant.
    #[tokio::test]
    async fn roundtrip_all_variants() {
        let cases = vec![
            Msg::Hello(Hello {
                protocol_version: PROTOCOL_VERSION,
                client_version: "0.1.0".into(),
                nonce: 42,
            }),
            Msg::Welcome(Welcome {
                protocol_version: PROTOCOL_VERSION,
                server_version: "0.1.0".into(),
                nonce: 42,
                accepted: true,
            }),
            sample_negotiate(),
            Msg::Start(Start {
                data_port: 5202,
                server_seed: 7,
            }),
            Msg::Run,
            Msg::Stop,
            sample_result(),
            Msg::Error(ProtoError::VersionMismatch),
        ];

        for original in cases {
            let mut buf = Vec::new();
            write_msg(&mut buf, &original).await.unwrap();
            let mut cursor = Cursor::new(buf);
            let decoded = read_msg(&mut cursor).await.unwrap();
            // Compare via JSON since Msg doesn't derive PartialEq.
            assert_eq!(
                serde_json::to_value(&original).unwrap(),
                serde_json::to_value(&decoded).unwrap(),
            );
        }
    }

    /// The frame is a 4-byte big-endian length prefix (§2) + JSON payload.
    #[tokio::test]
    async fn frame_has_be_length_prefix() {
        let msg = Msg::Run;
        let mut buf = Vec::new();
        write_msg(&mut buf, &msg).await.unwrap();

        let declared = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        assert_eq!(declared, buf.len() - 4, "prefix must equal payload length");
        assert_eq!(&buf[4..], br#"{"type":"Run"}"#);
    }

    /// A declared length over the cap is rejected without reading the body (§2).
    #[tokio::test]
    async fn oversized_declared_length_is_rejected() {
        let mut framed = (MAX_FRAME_BYTES + 1).to_be_bytes().to_vec();
        framed.extend_from_slice(b"ignored");
        let mut cursor = Cursor::new(framed);
        let err = read_msg(&mut cursor).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// Mode and RoleDir serialize as the lowercase tokens the spec shows.
    #[test]
    fn enum_wire_tokens() {
        assert_eq!(serde_json::to_string(&Mode::Tcp).unwrap(), r#""tcp""#);
        assert_eq!(serde_json::to_string(&Mode::Udp).unwrap(), r#""udp""#);
        assert_eq!(serde_json::to_string(&RoleDir::Send).unwrap(), r#""send""#);
        assert_eq!(
            serde_json::to_string(&ProtoError::DataConnectFailed).unwrap(),
            r#""data_connect_failed""#,
        );
    }
}

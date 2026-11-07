//! sidecar 本地命名管道的长度前缀 JSON 帧。

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// sidecar 的固定管道名。
pub const IPC_PIPE: &str = r"\\.\pipe\cloudime-mcp-client";

/// 单帧最大字节数，防止坏进程把 Server 内存撑爆。
const MAX_FRAME: usize = 8 * 1024 * 1024;

/// 写一帧：4 字节小端长度 + UTF-8 JSON。
pub fn write_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let body = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MCP IPC frame too large",
        ));
    }
    writer.write_all(&(body.len() as u32).to_le_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

/// 读一帧：4 字节小端长度 + UTF-8 JSON。
pub fn read_message<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MCP IPC frame too large",
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let message = serde_json::json!({ "id": 7, "action": { "type": "list_servers" } });
        let mut buffer = Vec::new();
        write_message(&mut buffer, &message).unwrap();
        let decoded: serde_json::Value = read_message(&mut &buffer[..]).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn rejects_a_frame_larger_than_the_limit() {
        let buffer = (MAX_FRAME as u32 + 1).to_le_bytes().to_vec();
        let result = read_message::<serde_json::Value>(&mut &buffer[..]);
        assert!(result.is_err());
    }
}

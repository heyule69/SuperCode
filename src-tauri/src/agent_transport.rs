//! Bounded JSONL framing shared by managed agents and diagnostic streams.
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

const MAX_FRAME: usize = 4 * 1024 * 1024;

pub(crate) async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut frame = Vec::new();
    loop {
        let buffer = reader.fill_buf().await.map_err(|error| error.to_string())?;
        if buffer.is_empty() {
            return if frame.is_empty() { Ok(None) } else { Err("Agent 返回了不完整的 JSONL 数据".into()) };
        }
        let end = buffer.iter().position(|byte| *byte == b'\n');
        let length = end.map_or(buffer.len(), |index| index + 1);
        if frame.len() + length > MAX_FRAME {
            return Err("Agent 单条事件超过 4 MB，连接已关闭以保护内存".into());
        }
        frame.extend_from_slice(&buffer[..length]);
        reader.consume(length);
        if end.is_some() { return Ok(Some(frame)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn fragmented_utf8_frames_preserve_newline_boundaries_and_reject_partial_eof() {
        let bytes = "{\"text\":\"你好\u{2028}🧋\"}\n{\"id\":2}\n".as_bytes();
        let mut reader = BufReader::with_capacity(4, bytes);
        let first = read_frame(&mut reader).await.unwrap().unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&first).is_ok());
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&read_frame(&mut reader).await.unwrap().unwrap()).unwrap()["id"], 2);
        assert!(read_frame(&mut reader).await.unwrap().is_none());
        let mut partial = BufReader::new("{\"id\":3}".as_bytes());
        assert!(read_frame(&mut partial).await.is_err());
    }

    #[tokio::test]
    async fn oversized_complete_or_unterminated_frames_are_rejected_at_the_same_bound() {
        for terminated in [false, true] {
            let mut bytes = vec![b'x'; MAX_FRAME + 1];
            if terminated { bytes.push(b'\n'); }
            let mut reader = BufReader::with_capacity(4096, bytes.as_slice());
            assert!(read_frame(&mut reader).await.unwrap_err().contains("4 MB"));
        }
    }
}

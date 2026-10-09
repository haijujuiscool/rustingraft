//! Frames: `u32` big-endian payload length, then the payload. An empty payload is a keepalive.

use std::io::{self, ErrorKind, Read, Write};

/// Largest payload a frame may carry. A longer length is a protocol error.
pub const MAX_FRAME: usize = 65_536;

/// Largest data payload a link can send: the relay adds a one-byte id or target to it.
pub const MAX_DATA: usize = MAX_FRAME - 1;

/// Reads one frame. Fails with `InvalidData` on a length over [`MAX_FRAME`] and with
/// `UnexpectedEof` on a truncated frame.
pub fn read_frame<R: Read + ?Sized>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 4];
    r.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(ErrorKind::InvalidData, "frame too large"));
    }
    let mut payload = vec![0; len];
    r.read_exact(&mut payload)?;
    Ok(payload)
}

/// Writes `payload` as one frame. Fails with `InvalidInput` if it is over [`MAX_FRAME`].
pub fn write_frame<W: Write + ?Sized>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    w.write_all(&encode_frame(&[], payload)?)
}

/// Header, `prefix` and `payload` in one buffer, so a frame goes out in a single write.
pub(crate) fn encode_frame(prefix: &[u8], payload: &[u8]) -> io::Result<Vec<u8>> {
    let len = prefix.len() + payload.len();
    if len > MAX_FRAME {
        return Err(io::Error::new(ErrorKind::InvalidInput, "frame too large"));
    }
    let mut frame = Vec::with_capacity(4 + len);
    frame.extend_from_slice(&(len as u32).to_be_bytes());
    frame.extend_from_slice(prefix);
    frame.extend_from_slice(payload);
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(result: io::Result<Vec<u8>>) -> ErrorKind {
        result.expect_err("frame should be rejected").kind()
    }

    #[test]
    fn round_trips_several_frames() {
        let mut wire = Vec::new();
        write_frame(&mut wire, b"hello").unwrap();
        write_frame(&mut wire, &[]).unwrap();
        write_frame(&mut wire, &[7; MAX_FRAME]).unwrap();
        let mut r = wire.as_slice();
        assert_eq!(read_frame(&mut r).unwrap(), b"hello");
        assert_eq!(read_frame(&mut r).unwrap(), Vec::<u8>::new());
        assert_eq!(read_frame(&mut r).unwrap().len(), MAX_FRAME);
        assert_eq!(kind(read_frame(&mut r)), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn zero_length_is_an_empty_keepalive() {
        assert_eq!(read_frame(&mut &[0u8, 0, 0, 0][..]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn rejects_oversized_lengths() {
        let header = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert_eq!(kind(read_frame(&mut &header[..])), ErrorKind::InvalidData);
        assert_eq!(kind(read_frame(&mut &[0xFF; 4][..])), ErrorKind::InvalidData);
        let err = write_frame(&mut Vec::new(), &[0; MAX_FRAME + 1]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_truncated_frames() {
        assert_eq!(kind(read_frame(&mut &[0u8, 0][..])), ErrorKind::UnexpectedEof);
        assert_eq!(kind(read_frame(&mut &[0u8, 0, 0, 5, 1, 2][..])), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn prefix_counts_toward_the_limit() {
        assert_eq!(encode_frame(&[3], &[0; MAX_DATA]).unwrap().len(), 4 + MAX_FRAME);
        assert!(encode_frame(&[3], &[0; MAX_FRAME]).is_err());
        assert_eq!(encode_frame(&[3], b"ab").unwrap(), [0, 0, 0, 3, 3, b'a', b'b']);
    }
}

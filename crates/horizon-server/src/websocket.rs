//! The server side of a WebSocket (RFC 6455) on a std `TcpStream`, for vanilla's management protocol
//! (`management.rs`): the handshake's accept key, and text frames in and out. Messages are aggregated up to
//! 64 KiB, as vanilla's `WebSocketFrameAggregator(65536)`; pings get their pong; binary frames are dropped,
//! as vanilla's pipeline only decodes text.

use base64::Engine;
use sha1::{Digest, Sha1};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    sync::Mutex,
};

/// What `Sec-WebSocket-Accept` answers for a `Sec-WebSocket-Key`.
pub fn accept_key(key: &str) -> String {
    let mut sha = Sha1::new();
    sha.update(key.trim().as_bytes());
    sha.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(sha.finalize())
}

/// The longest message read, as vanilla aggregates frames.
const MAX_MESSAGE: usize = 65536;

const TEXT: u8 = 1;
const CLOSE: u8 = 8;
const PING: u8 = 9;

/// One open WebSocket. Reads come from one thread; any thread may send.
pub struct Socket {
    reader: Mutex<TcpStream>,
    writer: Mutex<TcpStream>,
}

impl Socket {
    pub fn new(stream: &TcpStream) -> io::Result<Self> {
        stream.set_read_timeout(None)?;
        Ok(Self {
            reader: Mutex::new(stream.try_clone()?),
            writer: Mutex::new(stream.try_clone()?),
        })
    }

    /// The next text message, or `None` once the client closed the socket.
    pub fn read_text(&self) -> io::Result<Option<String>> {
        let mut stream = self.reader.lock().unwrap();
        let mut message = Vec::new();
        let mut text = false;
        loop {
            let mut head = [0u8; 2];
            if let Err(e) = stream.read_exact(&mut head) {
                return if e.kind() == io::ErrorKind::UnexpectedEof {
                    Ok(None)
                } else {
                    Err(e)
                };
            }
            let (fin, opcode) = (head[0] & 0x80 != 0, head[0] & 0x0f);
            if head[1] & 0x80 == 0 {
                return Err(invalid("a client's frame must be masked"));
            }
            let length = match head[1] & 0x7f {
                126 => {
                    let mut b = [0u8; 2];
                    stream.read_exact(&mut b)?;
                    u64::from(u16::from_be_bytes(b))
                }
                127 => {
                    let mut b = [0u8; 8];
                    stream.read_exact(&mut b)?;
                    u64::from_be_bytes(b)
                }
                n => u64::from(n),
            };
            if length > MAX_MESSAGE as u64 || message.len() + length as usize > MAX_MESSAGE {
                return Err(invalid("the message is longer than 65536 bytes"));
            }
            let mut mask = [0u8; 4];
            stream.read_exact(&mut mask)?;
            let mut payload = vec![0u8; length as usize];
            stream.read_exact(&mut payload)?;
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
            match opcode {
                CLOSE => {
                    let _ = self.send(CLOSE, &payload[..payload.len().min(2)]);
                    return Ok(None);
                }
                PING => self.send(0x0a, &payload)?,
                0x0a => {}
                0..=2 => {
                    if opcode != 0 {
                        message.clear();
                        text = opcode == TEXT;
                    }
                    message.extend_from_slice(&payload);
                    if fin && text {
                        return String::from_utf8(message)
                            .map(Some)
                            .map_err(|_| invalid("a text message must be UTF-8"));
                    }
                    if fin {
                        message = Vec::new();
                    }
                }
                _ => return Err(invalid("unknown frame opcode")),
            }
        }
    }

    pub fn send_text(&self, text: &str) -> io::Result<()> {
        self.send(TEXT, text.as_bytes())
    }

    /// Closes with a status code (1000: normal).
    pub fn close(&self, code: u16) {
        let _ = self.send(CLOSE, &code.to_be_bytes());
        let _ = self
            .writer
            .lock()
            .unwrap()
            .shutdown(std::net::Shutdown::Both);
    }

    fn send(&self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let mut frame = vec![0x80 | opcode];
        match payload.len() {
            n @ 0..=125 => frame.push(n as u8),
            n @ 126..=0xffff => {
                frame.push(126);
                frame.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                frame.push(127);
                frame.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(payload);
        let mut stream = self.writer.lock().unwrap();
        stream.write_all(&frame)?;
        stream.flush()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accept_key_is_the_rfcs() {
        // RFC 6455, section 1.3.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    /// A client's frame: masked, as browsers send them.
    fn frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [1u8, 2, 3, 4];
        let mut f = vec![if fin { 0x80 } else { 0 } | opcode];
        if payload.len() < 126 {
            f.push(0x80 | payload.len() as u8);
        } else {
            f.push(0x80 | 126);
            f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        f.extend_from_slice(&mask);
        f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        f
    }

    #[test]
    fn messages_come_whole_and_pings_get_pongs() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let socket = Socket::new(&listener.accept().unwrap().0).unwrap();
        let long = "x".repeat(300);
        let mut sent = frame(false, TEXT, b"{\"a\":");
        sent.extend(frame(true, PING, b"hi"));
        sent.extend(frame(true, 0, b"1}"));
        sent.extend(frame(true, 2, b"binary is dropped"));
        sent.extend(frame(true, TEXT, long.as_bytes()));
        sent.extend(frame(true, CLOSE, &1000u16.to_be_bytes()));
        client.write_all(&sent).unwrap();
        assert_eq!(socket.read_text().unwrap().as_deref(), Some("{\"a\":1}"));
        assert_eq!(socket.read_text().unwrap(), Some(long));
        assert_eq!(socket.read_text().unwrap(), None);
        socket.send_text("ok").unwrap();
        let mut got = [0u8; 8];
        client.read_exact(&mut got).unwrap();
        // The pong, then the close's echo; the text after it.
        assert_eq!(&got[..4], &[0x8a, 2, b'h', b'i']);
        assert_eq!(&got[4..8], &[0x88, 2, 0x03, 0xe8]);
        let mut text = [0u8; 4];
        client.read_exact(&mut text).unwrap();
        assert_eq!(&text, &[0x81, 2, b'o', b'k']);
    }

    #[test]
    fn unmasked_and_oversized_frames_are_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let socket = Socket::new(&listener.accept().unwrap().0).unwrap();
        client.write_all(&[0x81, 2, b'h', b'i']).unwrap();
        assert!(socket.read_text().is_err());
        let mut big = vec![0x81, 0x80 | 127];
        big.extend_from_slice(&(MAX_MESSAGE as u64 + 1).to_be_bytes());
        client.write_all(&big).unwrap();
        assert!(socket.read_text().is_err());
    }
}

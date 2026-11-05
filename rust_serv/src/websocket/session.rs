//! Message reassembly and echo semantics, with no socket or runtime dependency.
use super::{
    MAX_MESSAGE,
    codec::{Frame, Opcode, ProtocolError, validate_close},
};

#[derive(Debug, Default)]
pub struct Session {
    fragmented: Option<(Opcode, Vec<u8>)>,
    closing: bool,
    peer_closed: bool,
}
impl Session {
    pub fn is_closing(&self) -> bool {
        self.closing
    }
    pub fn peer_closed(&self) -> bool {
        self.peer_closed
    }
    pub fn start_close(&mut self, code: u16) -> Option<Frame> {
        if self.closing {
            return None;
        }
        self.closing = true;
        self.fragmented = None;
        Some(Frame::close(code))
    }
    /// Consume a client frame validated by `Frame::decode` and optionally reply.
    pub fn receive(&mut self, frame: Frame) -> Result<Option<Frame>, ProtocolError> {
        if frame.opcode == Opcode::Close {
            validate_close(&frame.payload)?;
            self.peer_closed = true;
            self.fragmented = None;
            if self.closing {
                return Ok(None);
            }
            self.closing = true;
            return Ok(Some(frame));
        }
        if self.closing {
            return Ok(None);
        }
        match frame.opcode {
            Opcode::Ping => Ok(Some(Frame {
                opcode: Opcode::Pong,
                ..frame
            })),
            Opcode::Pong => Ok(None),
            Opcode::Close => unreachable!(),
            Opcode::Text | Opcode::Binary => {
                if self.fragmented.is_some() {
                    return Err(ProtocolError::Protocol);
                }
                if frame.payload.len() > MAX_MESSAGE {
                    return Err(ProtocolError::TooLarge);
                }
                if frame.fin {
                    return Self::echo(frame.opcode, frame.payload).map(Some);
                }
                self.fragmented = Some((frame.opcode, frame.payload));
                Ok(None)
            }
            Opcode::Continuation => {
                let Some((opcode, payload)) = &mut self.fragmented else {
                    return Err(ProtocolError::Protocol);
                };
                if frame.payload.len() > MAX_MESSAGE - payload.len() {
                    return Err(ProtocolError::TooLarge);
                }
                payload.extend_from_slice(&frame.payload);
                if !frame.fin {
                    return Ok(None);
                }
                let opcode = *opcode;
                let payload = self.fragmented.take().unwrap().1;
                Self::echo(opcode, payload).map(Some)
            }
        }
    }
    fn echo(opcode: Opcode, payload: Vec<u8>) -> Result<Frame, ProtocolError> {
        if opcode == Opcode::Text {
            std::str::from_utf8(&payload).map_err(|_| ProtocolError::InvalidUtf8)?;
        }
        Ok(Frame {
            fin: true,
            opcode,
            payload,
        })
    }
}

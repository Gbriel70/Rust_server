//! Incremental client-frame decoding. Length checks precede payload allocation.
use super::MAX_FRAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}
impl Opcode {
    fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::Continuation),
            1 => Ok(Self::Text),
            2 => Ok(Self::Binary),
            8 => Ok(Self::Close),
            9 => Ok(Self::Ping),
            10 => Ok(Self::Pong),
            _ => Err(ProtocolError::Protocol),
        }
    }
    fn wire(self) -> u8 {
        match self {
            Self::Continuation => 0,
            Self::Text => 1,
            Self::Binary => 2,
            Self::Close => 8,
            Self::Ping => 9,
            Self::Pong => 10,
        }
    }
    fn control(self) -> bool {
        matches!(self, Self::Close | Self::Ping | Self::Pong)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    Protocol,
    InvalidUtf8,
    TooLarge,
}
impl ProtocolError {
    pub fn close_code(self) -> u16 {
        match self {
            Self::Protocol => 1002,
            Self::InvalidUtf8 => 1007,
            Self::TooLarge => 1009,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: Opcode,
    pub payload: Vec<u8>,
}
impl Frame {
    /// Accept only masked client frames. None means more bytes are required.
    pub fn decode(input: &[u8]) -> Result<Option<(Self, usize)>, ProtocolError> {
        if input.len() < 2 {
            return Ok(None);
        }
        let fin = input[0] & 0x80 != 0;
        let opcode = Opcode::from_wire(input[0] & 15)?;
        if input[0] & 0x70 != 0 || input[1] & 0x80 == 0 {
            return Err(ProtocolError::Protocol);
        }
        let short = input[1] & 127;
        if opcode.control() && (!fin || short > 125) {
            return Err(ProtocolError::Protocol);
        }
        let (length, header) = match short {
            126 => {
                if input.len() < 4 {
                    return Ok(None);
                }
                let length = u16::from_be_bytes([input[2], input[3]]) as u64;
                if length < 126 {
                    return Err(ProtocolError::Protocol);
                }
                (length, 4)
            }
            127 => {
                if input.len() < 10 {
                    return Ok(None);
                }
                let length = u64::from_be_bytes(input[2..10].try_into().unwrap());
                if length < 65536 || length >> 63 != 0 {
                    return Err(ProtocolError::Protocol);
                }
                (length, 10)
            }
            length => (u64::from(length), 2),
        };
        if length > MAX_FRAME as u64 {
            return Err(ProtocolError::TooLarge);
        }
        let length = length as usize;
        let end = header + 4 + length;
        if input.len() < end {
            return Ok(None);
        }
        let mask = &input[header..header + 4];
        let payload = input[header + 4..end]
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4])
            .collect();
        Ok(Some((
            Self {
                fin,
                opcode,
                payload,
            },
            end,
        )))
    }
    /// Server frames are always unmasked. Callers supply validated payloads.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.payload.len() + 10);
        bytes.push((u8::from(self.fin) * 0x80) | self.opcode.wire());
        match self.payload.len() {
            length @ 0..=125 => bytes.push(length as u8),
            length @ 126..=65535 => {
                bytes.push(126);
                bytes.extend_from_slice(&(length as u16).to_be_bytes());
            }
            length => {
                bytes.push(127);
                bytes.extend_from_slice(&(length as u64).to_be_bytes());
            }
        }
        bytes.extend_from_slice(&self.payload);
        bytes
    }
    pub fn close(code: u16) -> Self {
        Self {
            fin: true,
            opcode: Opcode::Close,
            payload: code.to_be_bytes().to_vec(),
        }
    }
}

pub fn validate_close(payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.is_empty() {
        return Ok(());
    }
    if payload.len() == 1 {
        return Err(ProtocolError::Protocol);
    }
    let code = u16::from_be_bytes([payload[0], payload[1]]);
    if !matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999) {
        return Err(ProtocolError::Protocol);
    }
    std::str::from_utf8(&payload[2..]).map_err(|_| ProtocolError::InvalidUtf8)?;
    Ok(())
}

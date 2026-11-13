//! Native voice packet framing. Cryptography is delegated to audited libraries;
//! these helpers never implement encryption or MLS.
use dc_protocol::Secret;
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("Malformed voice packet")]
    Packet,
    #[error("Unsupported voice transport or encryption mode")]
    Unsupported,
    #[error("Voice transport failed")]
    Transport,
    #[error("Voice encryption is not ready")]
    NotReady,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtpHeader {
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}
impl RtpHeader {
    pub fn encode(self) -> [u8; 12] {
        let mut header = [0u8; 12];
        header[0] = 0x80;
        header[1] = 0x78;
        header[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        header[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        header[8..12].copy_from_slice(&self.ssrc.to_be_bytes());
        header
    }
    /// The RTP header through CSRCs is authenticated by rtpsize transport
    /// encryption. An extension's four-byte preamble is authenticated too.
    pub fn parse(packet: &[u8]) -> Result<(Self, usize), VoiceError> {
        if packet.len() < 12 || packet[0] >> 6 != 2 || packet[1] & 0x7f != 0x78 {
            return Err(VoiceError::Packet);
        }
        let size =
            12 + usize::from(packet[0] & 0x0f) * 4 + if packet[0] & 0x10 != 0 { 4 } else { 0 };
        if size > packet.len() {
            return Err(VoiceError::Packet);
        }
        Ok((
            Self {
                sequence: u16::from_be_bytes([packet[2], packet[3]]),
                timestamp: u32::from_be_bytes(
                    packet[4..8].try_into().map_err(|_| VoiceError::Packet)?,
                ),
                ssrc: u32::from_be_bytes(packet[8..12].try_into().map_err(|_| VoiceError::Packet)?),
            },
            size,
        ))
    }
}
pub fn discovery_request(ssrc: u32) -> [u8; 74] {
    let mut data = [0; 74];
    data[1] = 1;
    data[3] = 70;
    data[4..8].copy_from_slice(&ssrc.to_be_bytes());
    data
}
pub fn discovery_response(data: &[u8], ssrc: u32) -> Result<(IpAddr, u16), VoiceError> {
    if data.len() != 74 || data[0..4] != [0, 2, 0, 70] || data[4..8] != ssrc.to_be_bytes() {
        return Err(VoiceError::Packet);
    }
    let end = data[8..72]
        .iter()
        .position(|b| *b == 0)
        .ok_or(VoiceError::Packet)?;
    let ip = std::str::from_utf8(&data[8..8 + end])
        .map_err(|_| VoiceError::Packet)?
        .parse()
        .map_err(|_| VoiceError::Packet)?;
    Ok((ip, u16::from_be_bytes([data[72], data[73]])))
}

/// A bounded reorder window. Sequence arithmetic works across u16 wraparound;
/// late packets and duplicates are ignored. Missing frames use codec PLC.
pub struct JitterBuffer {
    expected: Option<u16>,
    packets: HashMap<u16, Vec<u8>>,
    capacity: usize,
    primed: bool,
}
impl JitterBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            expected: None,
            packets: HashMap::new(),
            capacity: capacity.clamp(3, 100),
            primed: false,
        }
    }
    pub fn push(&mut self, sequence: u16, payload: Vec<u8>) {
        if payload.len() > 8192 {
            return;
        }
        let mut expected = *self.expected.get_or_insert(sequence);
        if !self.primed && expected.wrapping_sub(sequence) < self.capacity as u16 {
            expected = sequence;
            self.expected = Some(sequence);
            self.packets
                .retain(|seq, _| usize::from(seq.wrapping_sub(sequence)) < self.capacity);
        }
        let ahead = sequence.wrapping_sub(expected);
        if ahead >= 32768 {
            return;
        }
        if usize::from(ahead) >= self.capacity {
            self.packets.clear();
            self.expected = Some(sequence);
            self.primed = false;
        }
        if self.packets.len() < self.capacity {
            self.packets.entry(sequence).or_insert(payload);
        }
    }
    pub fn pop(&mut self) -> Option<Option<Vec<u8>>> {
        if !self.primed {
            if self.packets.len() < 3 {
                return None;
            }
            self.primed = true;
        }
        let expected = self.expected?;
        self.expected = Some(expected.wrapping_add(1));
        Some(self.packets.remove(&expected))
    }
}

/// Backend-only voice credentials. Neither this object nor its raw gateway
/// payload is ever forwarded to the UI or plugins.
#[derive(Debug)]
pub struct VoiceCredentials {
    pub guild_id: dc_protocol::Snowflake,
    pub channel_id: dc_protocol::Snowflake,
    pub user_id: dc_protocol::Snowflake,
    pub session_id: String,
    pub endpoint: String,
    pub token: Secret<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_roundtrip_and_short_packets_are_rejected() {
        let header = RtpHeader {
            sequence: 65535,
            timestamp: u32::MAX,
            ssrc: 42,
        };
        assert_eq!(RtpHeader::parse(&header.encode()).unwrap(), (header, 12));
        for length in 0..12 {
            assert!(RtpHeader::parse(&header.encode()[..length]).is_err());
        }
        let mut packet = header.encode();
        packet[0] |= 0x10;
        assert!(RtpHeader::parse(&packet).is_err());
    }
    #[test]
    fn discovery_checks_type_length_ssrc_and_address() {
        let mut data = discovery_request(42);
        data[1] = 2;
        data[8..17].copy_from_slice(b"127.0.0.1");
        data[72..].copy_from_slice(&5000u16.to_be_bytes());
        assert_eq!(
            discovery_response(&data, 42).unwrap(),
            ("127.0.0.1".parse().unwrap(), 5000)
        );
        assert!(discovery_response(&data, 43).is_err());
    }
    #[test]
    fn jitter_reorders_and_wraps_and_reports_a_missing_frame() {
        let mut buffer = JitterBuffer::new(10);
        buffer.push(65534, vec![1]);
        buffer.push(0, vec![3]);
        buffer.push(65535, vec![2]);
        assert_eq!(buffer.pop(), Some(Some(vec![1])));
        assert_eq!(buffer.pop(), Some(Some(vec![2])));
        assert_eq!(buffer.pop(), Some(Some(vec![3])));
        assert_eq!(buffer.pop(), Some(None));
        buffer.push(0, vec![9]);
        assert_eq!(buffer.pop(), Some(None));
    }
}

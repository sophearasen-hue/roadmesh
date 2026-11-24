//! Splitting radio frames over Bluetooth links with small packets (Step 6).
//!
//! iPhones cannot put a 218-byte frame in an advertisement, so they send frames as
//! GATT notifications to phones that connect to them. A notification holds MTU − 3
//! bytes, which can be as little as 20, so a frame is cut into fragments:
//!
//! `[frame_no:1][index:1][count:1][payload]`
//!
//! Untrusted bytes are reassembled only here (AUDIT S-09): every fragment is
//! length-checked, a frame never grows past [`crate::rsm`]'s maximum, and partial
//! frames are dropped when a new frame number starts.

/// Header bytes per fragment.
pub const HEADER: usize = 3;
/// Largest frame accepted: 2 + 8 + 31 + 1 + 176 (SPEC §3).
pub const MAX_FRAME: usize = 2 + 8 + crate::rsm::BODY_LEN + 1 + crate::rsm::MAX_AUTH_LEN;

/// Cut `frame` into fragments of at most `max_packet` bytes. `None` if the packet is too
/// small to make progress or the frame is too large.
pub fn split(frame: &[u8], frame_no: u8, max_packet: usize) -> Option<Vec<Vec<u8>>> {
    if frame.is_empty() || frame.len() > MAX_FRAME || max_packet <= HEADER {
        return None;
    }
    let chunk = max_packet - HEADER;
    let count = frame.len().div_ceil(chunk);
    let count_u8 = u8::try_from(count).ok()?;
    Some(
        frame
            .chunks(chunk)
            .enumerate()
            .map(|(i, part)| {
                let mut p = Vec::with_capacity(HEADER + part.len());
                p.push(frame_no);
                p.push(i as u8);
                p.push(count_u8);
                p.extend_from_slice(part);
                p
            })
            .collect(),
    )
}

/// Reassembles fragments from ONE link (one connected phone).
#[derive(Debug, Clone, Default)]
pub struct Reassembler {
    frame_no: Option<u8>,
    count: usize,
    parts: Vec<Option<Vec<u8>>>,
}

impl Reassembler {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one fragment (untrusted bytes). Returns a complete frame when the last part
    /// arrives. Malformed fragments are dropped.
    pub fn push(&mut self, packet: &[u8]) -> Option<Vec<u8>> {
        if packet.len() <= HEADER {
            return None;
        }
        let (no, idx, count) = (packet[0], usize::from(packet[1]), usize::from(packet[2]));
        if count == 0 || idx >= count {
            return None;
        }
        let payload = &packet[HEADER..];
        if self.frame_no != Some(no) || self.count != count {
            // a new frame: forget any unfinished one
            self.frame_no = Some(no);
            self.count = count;
            self.parts = vec![None; count];
        }
        if let Some(slot) = self.parts.get_mut(idx) {
            *slot = Some(payload.to_vec());
        }
        let total: usize = self.parts.iter().flatten().map(Vec::len).sum();
        if total > MAX_FRAME {
            self.reset();
            return None;
        }
        if self.parts.iter().all(Option::is_some) {
            let frame: Vec<u8> = self.parts.iter().flatten().flatten().copied().collect();
            self.reset();
            return Some(frame);
        }
        None
    }

    fn reset(&mut self) {
        self.frame_no = None;
        self.count = 0;
        self.parts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_reassemble_at_every_packet_size() {
        let frame: Vec<u8> = (0..218u16).map(|i| (i * 7) as u8).collect();
        for max in [20usize, 23, 64, 182, 244, 512] {
            let parts = split(&frame, 9, max).unwrap_or_default();
            assert!(parts.iter().all(|p| p.len() <= max));
            let mut r = Reassembler::new();
            let mut out = None;
            for p in &parts {
                out = r.push(p);
            }
            assert_eq!(out.as_deref(), Some(frame.as_slice()), "max {max}");
        }
    }

    #[test]
    fn out_of_order_parts_still_complete() {
        let frame = vec![5u8; 100];
        let mut parts = split(&frame, 1, 30).unwrap_or_default();
        parts.reverse();
        let mut r = Reassembler::new();
        let got: Vec<_> = parts.iter().filter_map(|p| r.push(p)).collect();
        assert_eq!(got, vec![frame]);
    }

    #[test]
    fn a_new_frame_drops_the_unfinished_one() {
        let a = split(&[1u8; 50], 1, 20).unwrap_or_default();
        let b = split(&[2u8; 30], 2, 20).unwrap_or_default();
        let mut r = Reassembler::new();
        assert!(r.push(&a[0]).is_none());
        assert!(r.push(&b[0]).is_none());
        assert_eq!(r.push(&b[1]), Some(vec![2u8; 30]));
        assert!(r.push(&a[1]).is_none()); // the rest of frame 1 does not resurrect it
    }

    #[test]
    fn malformed_or_oversize_input_is_rejected() {
        let mut r = Reassembler::new();
        for bad in [
            &[][..],
            &[1, 0, 1][..],
            &[1, 2, 2, 9][..],
            &[1, 0, 0, 9][..],
        ] {
            assert!(r.push(bad).is_none());
        }
        assert!(split(&[0u8; MAX_FRAME + 1], 0, 100).is_none());
        assert!(split(&[1, 2], 0, 3).is_none());
        // 255 parts of 200 bytes would exceed the maximum frame: dropped
        let mut r = Reassembler::new();
        let mut done = None;
        for i in 0..255u8 {
            let mut p = vec![7, i, 255];
            p.extend(vec![0u8; 200]);
            done = done.or(r.push(&p));
        }
        assert!(done.is_none());
    }
}

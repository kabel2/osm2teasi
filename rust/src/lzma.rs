//! Raw LZMA1 as the chart records use it: lc=3, lp=0, pb=2, 16 MiB dictionary,
//! no container header (Python: lzma.FORMAT_RAW with chart.LZMA_FILTER).
//!
//! liblzma itself cannot write a raw stream, only the ".lzma" (alone) format,
//! which is exactly a 13-byte header plus that raw stream: one properties byte,
//! the dictionary size and the uncompressed size.  So we encode as alone and
//! drop those 13 bytes, and prepend them again to decode.  Checked against
//! Python: byte-identical output.

use anyhow::{bail, Context, Result};
use xz2::stream::{Action, LzmaOptions, Status, Stream};

const LC: u32 = 3;
const LP: u32 = 0;
const PB: u32 = 2;
const DICT: u32 = 1 << 24;

/// The encoder needs about ten times its dictionary in memory, 190 MB at 16
/// MiB, and the writer runs one per thread.  A record never reaches back
/// further than its own length, so the encoder gets a dictionary just big
/// enough for it; the stream stays valid for the 16 MiB the decoder assumes.
fn options(len: usize) -> Result<LzmaOptions> {
    let mut o = LzmaOptions::new_preset(6).context("lzma preset")?;
    o.dict_size(len.next_power_of_two().clamp(4096, DICT as usize) as u32);
    o.literal_context_bits(LC);
    o.literal_position_bits(LP);
    o.position_bits(PB);
    Ok(o)
}

/// 13-byte ".lzma" header: properties, dictionary size and "size unknown".
///
/// The size stays unknown on purpose.  Our own records end with the end marker
/// liblzma appends, the original ones do not, and a stream of known size must
/// not carry that marker -- announcing the length would make liblzma reject one
/// of the two.  With the length unknown both are accepted, and the caller stops
/// after `pltx` bytes, exactly like the firmware's LzmaDecode.
fn alone_header() -> Vec<u8> {
    let props = ((PB * 5 + LP) * 9 + LC) as u8;
    let mut h = Vec::with_capacity(13);
    h.push(props);
    h.extend_from_slice(&DICT.to_le_bytes());
    h.extend_from_slice(&u64::MAX.to_le_bytes());
    h
}

/// Feed `input` through `s` until it ends or `limit` output bytes are there.
fn run(s: &mut Stream, input: &[u8], limit: Option<usize>) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(limit.unwrap_or(input.len() / 2 + 4096));
    let mut buf = vec![0u8; 256 * 1024];
    let mut pos = 0usize;
    loop {
        let (in0, out0) = (s.total_in(), s.total_out());
        let status = s.process(&input[pos..], &mut buf, Action::Finish)?;
        pos += (s.total_in() - in0) as usize;
        let got = (s.total_out() - out0) as usize;
        out.extend_from_slice(&buf[..got]);
        if let Some(n) = limit {
            if out.len() >= n {
                out.truncate(n);
                return Ok(out);
            }
        }
        match status {
            Status::StreamEnd => return Ok(out),
            Status::Ok => {}
            other => bail!("lzma: {:?}", other),
        }
        if got == 0 && pos == input.len() {
            bail!("lzma: stream ended early");
        }
    }
}

pub fn compress(raw: &[u8]) -> Result<Vec<u8>> {
    let mut s = Stream::new_lzma_encoder(&options(raw.len())?).context("lzma encoder")?;
    let mut out = run(&mut s, raw, None)?;
    if out.len() < 13 {
        bail!("lzma: output too short");
    }
    Ok(out.split_off(13))
}

/// `size` is the plaintext length from the record header (`pltx`).
pub fn decompress(data: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut input = alone_header();
    input.extend_from_slice(data);
    let mut s = Stream::new_lzma_decoder(u64::MAX).context("lzma decoder")?;
    let out = run(&mut s, &input, Some(size))?;
    if out.len() != size {
        bail!("lzma: {} of {} bytes", out.len(), size);
    }
    Ok(out)
}

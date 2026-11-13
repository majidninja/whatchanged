//! A small gzip reader (RFC 1952 + RFC 1951 inflate), std only.
//!
//! logrotate compresses old package logs (`dpkg.log.2.gz`), so looking
//! further back than a month or so means unpacking them. This is the plain,
//! bit-at-a-time way of decoding: slower than zlib, but rotated logs are
//! small and it keeps the tool free of dependencies.
//!
//! Every length, distance and checksum is checked, so a damaged or
//! truncated file gives an error and never a panic or wrong text.

/// Unpacks a whole gzip file (one or more members, as `cat a.gz b.gz`
/// gives). Stops with an error once the output would pass `limit` bytes.
pub fn gunzip(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let start = out.len();
        pos = header(data, pos)?;
        let mut bits = Bits {
            data,
            pos,
            buf: 0,
            cnt: 0,
        };
        inflate(&mut bits, &mut out, start, limit)?;
        // The trailer starts at the next whole byte.
        pos = bits.pos;
        let trailer = data.get(pos..pos + 8).ok_or("file is cut short")?;
        let word = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        if word(&trailer[..4]) != crc32(&out[start..]) {
            return Err("checksum doesn't match".into());
        }
        if word(&trailer[4..]) != (out.len() - start) as u32 {
            return Err("length doesn't match".into());
        }
        pos += 8;
        // Zero padding after the last member is allowed (tapes, some tools).
        if data[pos..].iter().all(|&b| b == 0) {
            return Ok(out);
        }
    }
}

/// Checks a member header at `pos` and returns where the data starts.
fn header(data: &[u8], pos: usize) -> Result<usize, String> {
    let h = data.get(pos..pos + 10).ok_or("file is cut short")?;
    if h[0] != 0x1f || h[1] != 0x8b {
        return Err("not a gzip file".into());
    }
    if h[2] != 8 {
        return Err("unknown compression method".into());
    }
    let flags = h[3];
    if flags & 0xe0 != 0 {
        return Err("unknown header flags".into());
    }
    let mut pos = pos + 10;
    let cut = || "file is cut short".to_string();
    if flags & 4 != 0 {
        // Extra field: two length bytes, then that many bytes.
        let len = data.get(pos..pos + 2).ok_or_else(cut)?;
        pos += 2 + usize::from(u16::from_le_bytes([len[0], len[1]]));
    }
    for flag in [8, 16] {
        // File name, then comment: each ends with a zero byte.
        if flags & flag != 0 {
            let rest = data.get(pos..).ok_or_else(cut)?;
            pos += rest.iter().position(|&b| b == 0).ok_or_else(cut)? + 1;
        }
    }
    if flags & 2 != 0 {
        pos += 2; // header checksum
    }
    if pos > data.len() {
        return Err(cut());
    }
    Ok(pos)
}

/// Reads bits from the input, lowest bit of each byte first.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
}

impl Bits<'_> {
    /// The next `n` bits (at most 16) as a number.
    fn take(&mut self, n: u32) -> Result<u32, String> {
        while self.cnt < n {
            let byte = *self.data.get(self.pos).ok_or("file is cut short")?;
            self.pos += 1;
            self.buf |= u32::from(byte) << self.cnt;
            self.cnt += 8;
        }
        let value = self.buf & ((1 << n) - 1);
        self.buf >>= n;
        self.cnt -= n;
        Ok(value)
    }

    /// Drops the bits left over from the current byte.
    fn align(&mut self) {
        self.buf = 0;
        self.cnt = 0;
    }
}

/// A canonical Huffman code: how many codes there are of each length, and
/// the symbols in code order.
struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    /// Builds the code from each symbol's code length (0 = unused).
    /// Returns the code and whether it is complete (every bit pattern
    /// leads to a symbol).
    fn new(lengths: &[u8]) -> Result<(Huffman, bool), String> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[usize::from(l)] += 1;
        }
        // More codes of some length than bit patterns left is nonsense.
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left = (left << 1) - i32::from(c);
            if left < 0 {
                return Err("damaged data (bad code lengths)".into());
            }
        }
        let mut offset = [0u16; 16];
        for len in 1..15 {
            offset[len + 1] = offset[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[usize::from(offset[usize::from(l)])] = sym as u16;
                offset[usize::from(l)] += 1;
            }
        }
        count[0] = 0;
        Ok((Huffman { count, symbol }, left == 0))
    }

    /// Reads one symbol, a bit at a time.
    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.count[len]);
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("damaged data (bad code)".into())
    }
}

/// Base length and extra bits for length symbols 257..=285.
const LENGTHS: [(u16, u8); 29] = [
    (3, 0),
    (4, 0),
    (5, 0),
    (6, 0),
    (7, 0),
    (8, 0),
    (9, 0),
    (10, 0),
    (11, 1),
    (13, 1),
    (15, 1),
    (17, 1),
    (19, 2),
    (23, 2),
    (27, 2),
    (31, 2),
    (35, 3),
    (43, 3),
    (51, 3),
    (59, 3),
    (67, 4),
    (83, 4),
    (99, 4),
    (115, 4),
    (131, 5),
    (163, 5),
    (195, 5),
    (227, 5),
    (258, 0),
];

/// Base distance and extra bits for distance symbols 0..=29.
const DISTANCES: [(u16, u8); 30] = [
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (5, 1),
    (7, 1),
    (9, 2),
    (13, 2),
    (17, 3),
    (25, 3),
    (33, 4),
    (49, 4),
    (65, 5),
    (97, 5),
    (129, 6),
    (193, 6),
    (257, 7),
    (385, 7),
    (513, 8),
    (769, 8),
    (1025, 9),
    (1537, 9),
    (2049, 10),
    (3073, 10),
    (4097, 11),
    (6145, 11),
    (8193, 12),
    (12289, 12),
    (16385, 13),
    (24577, 13),
];

/// Order in which a dynamic block stores its code length code lengths.
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Unpacks one deflate stream onto the end of `out`. `start` is where this
/// member's output begins: a copy may not reach back before it.
fn inflate(bits: &mut Bits, out: &mut Vec<u8>, start: usize, limit: usize) -> Result<(), String> {
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => stored(bits, out, limit)?,
            1 => {
                let mut lengths = [8u8; 288];
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                let (lit, _) = Huffman::new(&lengths)?;
                let (dist, _) = Huffman::new(&[5u8; 30])?;
                codes(bits, out, start, limit, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_tables(bits)?;
                codes(bits, out, start, limit, &lit, &dist)?;
            }
            _ => return Err("damaged data (bad block type)".into()),
        }
        if last {
            bits.align();
            return Ok(());
        }
    }
}

/// A block of bytes stored as they are.
fn stored(bits: &mut Bits, out: &mut Vec<u8>, limit: usize) -> Result<(), String> {
    bits.align();
    let head = bits
        .data
        .get(bits.pos..bits.pos + 4)
        .ok_or("file is cut short")?;
    let len = u16::from_le_bytes([head[0], head[1]]);
    if len != !u16::from_le_bytes([head[2], head[3]]) {
        return Err("damaged data (bad stored block)".into());
    }
    bits.pos += 4;
    let len = usize::from(len);
    let bytes = bits
        .data
        .get(bits.pos..bits.pos + len)
        .ok_or("file is cut short")?;
    if out.len() + len > limit {
        return Err(too_big(limit));
    }
    out.extend_from_slice(bytes);
    bits.pos += len;
    Ok(())
}

/// Reads the two Huffman codes at the start of a dynamic block.
fn dynamic_tables(bits: &mut Bits) -> Result<(Huffman, Huffman), String> {
    let bad = || "damaged data (bad code lengths)".to_string();
    let nlen = bits.take(5)? as usize + 257;
    let ndist = bits.take(5)? as usize + 1;
    let ncode = bits.take(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(bad());
    }
    let mut lengths = [0u8; 19];
    for &slot in &ORDER[..ncode] {
        lengths[slot] = bits.take(3)? as u8;
    }
    let (lencode, complete) = Huffman::new(&lengths)?;
    if !complete {
        return Err(bad());
    }
    // Literal/length and distance code lengths, run-length coded as one list.
    let mut lengths = [0u8; 286 + 30];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = lencode.decode(bits)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => {
                let prev = *lengths.get(i.wrapping_sub(1)).ok_or_else(bad)?;
                (prev, 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            _ => (0, 11 + bits.take(7)? as usize),
        };
        if i + repeat > nlen + ndist {
            return Err(bad());
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    if lengths[256] == 0 {
        return Err(bad()); // no end-of-block code
    }
    // An incomplete code is only allowed when it has at most one symbol
    // (a block with one distance, or with literals only).
    let (lit, complete) = Huffman::new(&lengths[..nlen])?;
    if !complete && lit.used() > 1 {
        return Err(bad());
    }
    let (dist, complete) = Huffman::new(&lengths[nlen..nlen + ndist])?;
    if !complete && dist.used() > 1 {
        return Err(bad());
    }
    Ok((lit, dist))
}

impl Huffman {
    /// How many symbols have a code.
    fn used(&self) -> usize {
        self.count.iter().map(|&c| usize::from(c)).sum()
    }
}

/// Unpacks literals and back-references until the end-of-block symbol.
fn codes(
    bits: &mut Bits,
    out: &mut Vec<u8>,
    start: usize,
    limit: usize,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), String> {
    loop {
        let sym = usize::from(lit.decode(bits)?);
        match sym {
            0..=255 => {
                if out.len() >= limit {
                    return Err(too_big(limit));
                }
                out.push(sym as u8);
            }
            256 => return Ok(()),
            _ => {
                let &(base, extra) = LENGTHS.get(sym - 257).ok_or("damaged data (bad length)")?;
                let len = usize::from(base) + bits.take(u32::from(extra))? as usize;
                let &(base, extra) = DISTANCES
                    .get(usize::from(dist.decode(bits)?))
                    .ok_or("damaged data (bad distance)")?;
                let distance = usize::from(base) + bits.take(u32::from(extra))? as usize;
                if distance > out.len() - start {
                    return Err("damaged data (distance too far back)".into());
                }
                if out.len() + len > limit {
                    return Err(too_big(limit));
                }
                // Byte by byte: the copy may overlap what it is writing.
                for _ in 0..len {
                    out.push(out[out.len() - distance]);
                }
            }
        }
    }
}

fn too_big(limit: usize) -> String {
    format!("unpacks to more than {} MB", limit >> 20)
}

/// The CRC-32 used by gzip.
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    !data.iter().fold(!0u32, |crc, &b| {
        table[((crc ^ u32::from(b)) & 0xff) as usize] ^ (crc >> 8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// The text the fixtures below were made from.
    fn log(lines: usize) -> String {
        (0..lines)
            .map(|i| {
                format!(
                    "2024-05-{:02} 09:{:02}:33 upgrade pkg{i}:amd64 1.{i}-1 1.{}-1\n",
                    i % 28 + 1,
                    i % 60,
                    i + 1
                )
            })
            .collect()
    }

    const FIXED: &str = concat!(
        "1f8b080000000000020333323032d13530d535305430b0b43230b0323656282d",
        "482f4a4c495528c84e37b04acc4d31335130d433d035049286ba865c46302d46",
        "602d86685a0ce15a0cc15a8c805a00d56f1ae566000000",
    );
    const DYNAMIC: &str = concat!(
        "1f8b080000000000020365d64b8e14510c44d139aba80d147a11b6f353bb69a9",
        "510f10520b89fd03769236f6246777f48e22cd457d2e7b2e3cd6f95aeb25f2f8",
        "f5f9f1f3edfddbe3f3fbc77abdfd78dff481afeb893f5f3cf185ff127a8296e0",
        "4ee0096b229eb025bc137a2235514fa4257227e289d6c43cd196e89da8275693",
        "cd136b89dd8979b2d564f7646bc976279b277b4d0e4ff696ec77b27b72d4e4f4",
        "e468c9712787276749b03c395b72dec9194fb96ae3cf8ffefcc8f7c705a00a80",
        "0bc0105008840154047004e808900a100c501dc01da03b4042404840a500a780",
        "4e016901810155035c03ba062407840754107010e82090221024504dc04da09b",
        "40a240a84065016781ce02e902010355065d06ba0c240d840d561b741bec3698",
        "361836586dd06db0db60dae0b50fd506dd06c7409485081bac36e836d86d306d",
        "306cb0daa0db60b7c1b4c1b0c16a836e83dd06d306c306ab0dba0d761b4c1b0c",
        "1bac36e836d86d306d306cf0bfc988f7ea3698361836788e3567b7c1b4c1b021",
        "6b0cba741b9236246c08c6a64bb7216943c28670ccba741b9236e4fa7dc85876",
        "19ff8ff203091ba263dca5db90b42161436cecbb741b9236246cc836265eba0d",
        "491b1236641f2b2fdd86a40d091b728c9d976e43d286840d39c7ce4bb7216943",
        "c286aeb1f3da6d68dad0b0a1183bafdd86a60d0d1bcab1f3da6d68dad0b0a132",
        "765ebb0d4d1b7a5d173a765ec77951ee8bb0a136765ebb0d4d1b1a36741b3baf",
        "dd86a60d0d1bba8f9dd76e43d386860d3dc6ce6bb7a16943c3869e63e7b5dbd0",
        "b4a161c3d6d879eb362c6d58d8308c9db76ec3d286850de3d879eb362c6d58d8",
        "30193b6fdd86a50d0b1ba663e7addbb0b461d7f16963e76d5c9fe5fc0c1bb68d",
        "9db76ec3d286850ddbc7ce5bb76169c3c2861d63e7addbb0b46161c3ceb1f3d6",
        "6d58dab0b0b1fdb5f11b1603cb6e8b0c0000",
    );
    const STORED: &str = concat!(
        "1f8b080000000000040301660099ff323032342d30352d30312030393a30303a",
        "3333207570677261646520706b67303a616d64363420312e302d3120312e312d",
        "310a323032342d30352d30322030393a30313a3333207570677261646520706b",
        "67313a616d64363420312e312d3120312e322d310ad56f1ae566000000",
    );
    const EMPTY: &str = "1f8b080000000000020303000000000000000000";
    const NAMED: &str = concat!(
        "1f8b08085974f76a020364706b672e6c6f672e320033323032d13530d5353054",
        "30b0b43230b0323656282d482f4a4c495528c84e37b04acc4d31335130d433d0",
        "35049286ba865c46302d46602d86685a0ce15a0cc15a8c805a00d56f1ae56600",
        "0000",
    );

    #[test]
    fn unpacks_fixed_dynamic_and_stored_blocks() {
        assert_eq!(gunzip(&hex(FIXED), 1 << 20).unwrap(), log(2).as_bytes());
        assert_eq!(gunzip(&hex(DYNAMIC), 1 << 20).unwrap(), log(60).as_bytes());
        assert_eq!(gunzip(&hex(STORED), 1 << 20).unwrap(), log(2).as_bytes());
        assert_eq!(gunzip(&hex(EMPTY), 1 << 20).unwrap(), b"");
    }

    #[test]
    fn reads_header_fields_and_several_members() {
        // Made by gzip itself, so the header carries the file name.
        assert_eq!(gunzip(&hex(NAMED), 1 << 20).unwrap(), log(2).as_bytes());
        let mut two = hex(FIXED);
        two.extend(hex(DYNAMIC));
        two.extend([0, 0, 0]);
        let both = log(2) + &log(60);
        assert_eq!(gunzip(&two, 1 << 20).unwrap(), both.as_bytes());
    }

    #[test]
    fn damaged_files_are_errors_not_panics() {
        for fixture in [FIXED, DYNAMIC, STORED, NAMED] {
            let good = hex(fixture);
            for cut in 0..good.len() {
                assert!(gunzip(&good[..cut], 1 << 20).is_err(), "cut at {cut}");
            }
            // Any flipped bit is either caught or (in unused header bytes
            // such as the time stamp) changes nothing.
            let want = gunzip(&good, 1 << 20).unwrap();
            for i in 0..good.len() {
                for bit in 0..8 {
                    let mut bad = good.clone();
                    bad[i] ^= 1 << bit;
                    if let Ok(got) = gunzip(&bad, 1 << 20) {
                        assert_eq!(got, want, "byte {i} bit {bit}");
                    }
                }
            }
        }
        assert_eq!(
            gunzip(b"plain text, not gzip", 100).unwrap_err(),
            "not a gzip file"
        );
    }

    #[test]
    fn stops_at_the_size_limit() {
        let err = gunzip(&hex(DYNAMIC), 1000).unwrap_err();
        assert!(err.contains("more than"), "{err}");
        assert!(gunzip(&hex(STORED), 10).is_err());
        let exact = log(60).len();
        assert!(gunzip(&hex(DYNAMIC), exact).is_ok());
        assert!(gunzip(&hex(DYNAMIC), exact - 1).is_err());
    }

    #[test]
    fn crc32_matches_the_known_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    /// Compares against the system's gzip on whatever .gz files this
    /// machine has. Not run by default: tests must not depend on the box.
    /// `cargo test -- --ignored matches_system_gzip`
    #[test]
    #[ignore]
    fn matches_system_gzip() {
        let mut checked = 0;
        let mut dirs = vec![
            std::path::PathBuf::from("/var/log"),
            std::path::PathBuf::from("/usr/share/doc"),
            std::path::PathBuf::from("/usr/share/man"),
            std::path::PathBuf::from("/usr/share/info"),
        ];
        while let Some(dir) = dirs.pop() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = e.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    dirs.push(path);
                } else if meta.is_file() && path.extension().is_some_and(|x| x == "gz") {
                    let Ok(data) = std::fs::read(&path) else {
                        continue;
                    };
                    let want = std::process::Command::new("gzip")
                        .arg("-dc")
                        .arg(&path)
                        .output()
                        .unwrap();
                    if !want.status.success() {
                        continue;
                    }
                    let got = gunzip(&data, usize::MAX).unwrap_or_else(|e| panic!("{path:?}: {e}"));
                    assert!(got == want.stdout, "{path:?} differs");
                    checked += 1;
                }
            }
        }
        println!("checked {checked} files");
        assert!(checked > 0);
    }
}

//! Décompression `.xz` : conteneur xz lu ici, données LZMA2 décodées par
//! `lzma-rs`, filtre **BCJ x86** appliqué ici (lzma-rs ne gère que LZMA2 seul ;
//! l'archive Linux officielle de 7-Zip utilise BCJ x86 + LZMA2).
//!
//! Les sommes de contrôle internes (CRC/SHA) ne sont pas revérifiées : le
//! fichier entier a déjà été vérifié par son SHA-256 épinglé avant extraction.
//! Algorithme BCJ x86 : celui de xz-embedded (domaine public).

use std::io::{self, BufRead, Read, Write};

use anyhow::{Context, Result, bail};

const MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const FILTER_X86: u64 = 0x04;
const FILTER_LZMA2: u64 = 0x21;

/// Lecteur qui compte les octets consommés (alignement des blocs).
struct Counting<R> {
    inner: R,
    count: u64,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count += n as u64;
        Ok(n)
    }
}

impl<R: BufRead> BufRead for Counting<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.count += amt as u64;
        self.inner.consume(amt);
    }
}

fn read_exact<R: Read>(r: &mut R, n: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0; n];
    r.read_exact(&mut buf).context("fichier xz tronqué")?;
    Ok(buf)
}

/// Entier variable xz (7 bits par octet, au plus 9 octets).
fn varint(buf: &[u8], pos: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for i in 0..9 {
        let b = *buf.get(*pos).context("en-tête de bloc xz invalide")?;
        *pos += 1;
        value |= u64::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("entier xz invalide")
}

/// Chaîne de filtres d'un en-tête de bloc → `Some(décalage BCJ)` si BCJ x86.
fn block_filters(header: &[u8]) -> Result<Option<u32>> {
    let flags = header[0];
    let mut pos = 1;
    for present in [flags & 0x40 != 0, flags & 0x80 != 0] {
        if present {
            varint(header, &mut pos)?; // tailles compressée / décompressée
        }
    }
    let mut ids = Vec::new();
    let mut start = 0u32;
    for _ in 0..=(flags & 0x03) {
        let id = varint(header, &mut pos)?;
        let size = varint(header, &mut pos)? as usize;
        let props = header
            .get(pos..pos + size)
            .context("propriétés de filtre xz tronquées")?;
        if id == FILTER_X86 && size == 4 {
            start = u32::from_le_bytes([props[0], props[1], props[2], props[3]]);
        }
        pos += size;
        ids.push(id);
    }
    match ids[..] {
        [FILTER_LZMA2] => Ok(None),
        [FILTER_X86, FILTER_LZMA2] => Ok(Some(start)),
        _ => bail!("filtres xz non pris en charge : {ids:?}"),
    }
}

/// Taille des sommes de contrôle de bloc selon le type de contrôle.
fn check_size(kind: u8) -> Result<u64> {
    Ok(match kind {
        0x00 => 0,
        0x01 => 4,
        0x04 => 8,
        0x0A => 32,
        other => bail!("type de contrôle xz inconnu : {other}"),
    })
}

/// Décompresse tout le flux xz `input` vers `output`.
pub fn decompress<R: BufRead, W: Write>(input: R, output: &mut W) -> Result<()> {
    let mut r = Counting {
        inner: input,
        count: 0,
    };
    let header = read_exact(&mut r, 12)?;
    if header[..6] != MAGIC {
        bail!("ce n'est pas un fichier xz");
    }
    let check = check_size(header[7] & 0x0F)?;
    loop {
        let start = r.count;
        let first = read_exact(&mut r, 1)?[0];
        if first == 0 {
            return Ok(()); // index : fin des blocs
        }
        let rest = read_exact(&mut r, usize::from(first) * 4 + 3)?;
        let bcj = block_filters(&rest)?;
        let mut data = Vec::new();
        lzma_rs::lzma2_decompress(&mut r, &mut data)
            .map_err(|e| anyhow::anyhow!("décompression LZMA2 impossible : {e:?}"))?;
        if let Some(offset) = bcj {
            bcj_x86_decode(&mut data, offset);
        }
        output.write_all(&data)?;
        let used = r.count - start;
        let padding = (4 - used % 4) % 4;
        read_exact(&mut r, (padding + check) as usize)?;
    }
}

const ALLOWED: [bool; 8] = [true, true, true, false, true, false, false, false];
const BIT_NUM: [u32; 8] = [0, 1, 2, 2, 3, 3, 3, 3];

fn ms_byte(b: u8) -> bool {
    b == 0x00 || b == 0xFF
}

/// Inverse du filtre BCJ x86 (adresses absolues des CALL/JMP → relatives).
pub fn bcj_x86_decode(buf: &mut [u8], start: u32) {
    if buf.len() <= 4 {
        return;
    }
    let size = buf.len() - 4;
    let mut prev_pos = usize::MAX;
    let mut prev_mask: u32 = 0;
    let mut i = 0;
    while i < size {
        if buf[i] & 0xFE != 0xE8 {
            i += 1;
            continue;
        }
        let gap = i.wrapping_sub(prev_pos);
        prev_pos = i;
        if gap > 3 {
            prev_mask = 0;
        } else {
            prev_mask = (prev_mask << (gap - 1)) & 7;
            if prev_mask != 0 {
                let b = buf[i + 4 - BIT_NUM[prev_mask as usize] as usize];
                if !ALLOWED[prev_mask as usize] || ms_byte(b) {
                    prev_mask = (prev_mask << 1) | 1;
                    i += 1;
                    continue;
                }
            }
        }
        if ms_byte(buf[i + 4]) {
            convert_at(buf, i, start, prev_mask);
            i += 5;
        } else {
            prev_mask = (prev_mask << 1) | 1;
            i += 1;
        }
    }
}

fn convert_at(buf: &mut [u8], i: usize, start: u32, prev_mask: u32) {
    let mut src = u32::from_le_bytes([buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4]]);
    let pos = start.wrapping_add(i as u32).wrapping_add(5);
    let mut dest;
    loop {
        dest = src.wrapping_sub(pos);
        if prev_mask == 0 {
            break;
        }
        let j = BIT_NUM[prev_mask as usize] * 8;
        if !ms_byte((dest >> (24 - j)) as u8) {
            break;
        }
        src = dest ^ ((1u32 << (32 - j)) - 1);
    }
    dest &= 0x01FF_FFFF;
    dest |= 0u32.wrapping_sub(dest & 0x0100_0000);
    buf[i + 1..i + 5].copy_from_slice(&dest.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn lzma2_seul() {
        let data = b"bonjour ".repeat(1000);
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut &data[..], &mut xz).unwrap();
        let mut out = Vec::new();
        decompress(&xz[..], &mut out).unwrap();
        assert_eq!(out, data);
        assert!(decompress(&b"pas du xz"[..], &mut Vec::new()).is_err());
    }

    /// Compression BCJ x86 + LZMA2 par l'outil `xz` du système (ignoré s'il manque),
    /// sur des données pleines d'instructions CALL (0xE8) : relecture identique.
    #[cfg(unix)]
    #[test]
    fn bcj_x86_avec_l_outil_xz() {
        let Some(xz) = which_xz() else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let mut data = Vec::new();
        for k in 0u32..20_000 {
            data.extend_from_slice(&[0x55, 0xE8]);
            data.extend_from_slice(&(k.wrapping_mul(2_654_435_761) % 4096).to_le_bytes());
            data.extend_from_slice(&[0x90, 0xE9, 0x00, 0x00, 0x00, 0x00, 0xFF]);
        }
        let raw = tmp.path().join("d.bin");
        std::fs::write(&raw, &data).unwrap();
        let args = ["--x86", "--lzma2", "-k", "-f"];
        let mut all: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
        all.push(raw.clone().into());
        crate::sidecar::run(&xz, &all, Duration::from_secs(30))
            .unwrap()
            .check("xz")
            .unwrap();
        let packed = std::fs::read(tmp.path().join("d.bin.xz")).unwrap();
        assert!(
            lzma_rs::xz_decompress(&mut &packed[..], &mut Vec::new()).is_err(),
            "lzma-rs seul échoue"
        );
        let mut out = Vec::new();
        decompress(&packed[..], &mut out).unwrap();
        assert!(out == data, "données BCJ mal décodées");
    }

    fn which_xz() -> Option<std::path::PathBuf> {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|d| d.join("xz"))
            .find(|p| crate::sidecar::is_executable(p))
    }
}

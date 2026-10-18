//! Cut a General MIDI soundfont down to the presets the tracks play, so the bank we ship
//! stays small: `cargo run --release --example trim_soundfont -- in.sf2 out.sf2 0:0 0:32 128:0`.
//!
//! Each `bank:program` keeps that preset with its instruments and samples; everything else
//! (and the sample data only it used) is dropped. For a drum kit, `128:0=35,51,62` also drops
//! the kit's zones that play none of the listed keys.

use std::collections::BTreeMap;

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The chunks of a RIFF list body, by id.
fn chunks(body: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= body.len() {
        let id = [body[i], body[i + 1], body[i + 2], body[i + 3]];
        let len = u32le(body, i + 4) as usize;
        out.push((id, &body[i + 8..i + 8 + len]));
        i += 8 + len + (len & 1);
    }
    out
}

fn find<'a>(list: &[([u8; 4], &'a [u8])], id: &[u8; 4]) -> &'a [u8] {
    list.iter().find(|(i, _)| i == id).map(|(_, b)| *b).unwrap_or_else(|| panic!("no {} chunk", String::from_utf8_lossy(id)))
}

/// A LIST chunk's form type and sub-chunks.
fn list<'a>(top: &[([u8; 4], &'a [u8])], form: &[u8; 4]) -> &'a [u8] {
    for (id, b) in top {
        if id == b"LIST" && &b[..4] == form {
            return &b[4..];
        }
    }
    panic!("no {} list", String::from_utf8_lossy(form));
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() & 1 == 1 {
        out.push(0);
    }
}

fn list_chunk(form: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = form.to_vec();
    b.extend_from_slice(body);
    let mut out = Vec::new();
    chunk(&mut out, b"LIST", &b);
    out
}

/// Fixed-size records (the last is the terminal one).
fn records(b: &[u8], size: usize) -> Vec<&[u8]> {
    b.chunks_exact(size).collect()
}

const GEN_KEY_RANGE: u16 = 43;
const GEN_INSTRUMENT: u16 = 41;
const GEN_SAMPLE: u16 = 53;

struct Zone {
    gens: Vec<[u8; 4]>,
    mods: Vec<[u8; 10]>,
}

/// Bags `[from, to)` as zones of generators and modulators.
fn zones(bag: &[&[u8]], gen_: &[&[u8]], mod_: &[&[u8]], from: usize, to: usize) -> Vec<Zone> {
    (from..to)
        .map(|z| {
            let (g0, m0) = (u16le(bag[z], 0) as usize, u16le(bag[z], 2) as usize);
            let (g1, m1) = (u16le(bag[z + 1], 0) as usize, u16le(bag[z + 1], 2) as usize);
            Zone {
                gens: gen_[g0..g1].iter().map(|g| g[..4].try_into().unwrap()).collect(),
                mods: mod_[m0..m1].iter().map(|m| m[..10].try_into().unwrap()).collect(),
            }
        })
        .collect()
}

fn gen_of(z: &Zone, oper: u16) -> Option<u16> {
    z.gens.iter().find(|g| u16le(&g[..], 0) == oper).map(|g| u16le(&g[..], 2))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: trim_soundfont in.sf2 out.sf2 bank:program[=key,key...]...");
        std::process::exit(2);
    }
    let mut keep: BTreeMap<(u16, u16), Option<Vec<u8>>> = BTreeMap::new();
    for a in &args[2..] {
        let (bp, keys) = a.split_once('=').map(|(x, k)| (x, Some(k))).unwrap_or((a, None));
        let (b, p) = bp.split_once(':').expect("bank:program");
        let keys = keys.map(|k| k.split(',').map(|n| n.parse().expect("a key 0..127")).collect());
        keep.insert((b.parse().unwrap(), p.parse().unwrap()), keys);
    }

    let file = std::fs::read(&args[0]).expect("read the soundfont");
    assert!(&file[..4] == b"RIFF" && &file[8..12] == b"sfbk", "not a soundfont");
    let top = chunks(&file[12..]);
    let info = list(&top, b"INFO");
    let smpl = find(&chunks(list(&top, b"sdta")), b"smpl");
    let pdta = chunks(list(&top, b"pdta"));
    let phdr = records(find(&pdta, b"phdr"), 38);
    let pbag = records(find(&pdta, b"pbag"), 4);
    let pmod = records(find(&pdta, b"pmod"), 10);
    let pgen = records(find(&pdta, b"pgen"), 4);
    let inst = records(find(&pdta, b"inst"), 22);
    let ibag = records(find(&pdta, b"ibag"), 4);
    let imod = records(find(&pdta, b"imod"), 10);
    let igen = records(find(&pdta, b"igen"), 4);
    let shdr = records(find(&pdta, b"shdr"), 46);

    // Presets to keep, and which instruments (and which of their keys) they need.
    let mut presets = Vec::new();
    let mut want_inst: BTreeMap<usize, Option<Vec<u8>>> = BTreeMap::new();
    for p in 0..phdr.len() - 1 {
        let (program, bank) = (u16le(phdr[p], 20), u16le(phdr[p], 22));
        let Some(keys) = keep.remove(&(bank, program)) else { continue };
        let zs = zones(&pbag, &pgen, &pmod, u16le(phdr[p], 24) as usize, u16le(phdr[p + 1], 24) as usize);
        for z in &zs {
            if let Some(i) = gen_of(z, GEN_INSTRUMENT) {
                let entry = want_inst.entry(i as usize).or_insert(Some(Vec::new()));
                match (entry.as_mut(), &keys) {
                    (Some(v), Some(k)) => v.extend(k),
                    _ => *entry = None,
                }
            }
        }
        presets.push((p, zs));
    }
    if let Some(((b, p), _)) = keep.iter().next() {
        panic!("no preset {b}:{p} in {}", args[0]);
    }

    // Instruments, keeping only the zones that can sound one of the wanted keys.
    let plays = |z: &Zone, keys: &Option<Vec<u8>>| match (keys, gen_of(z, GEN_KEY_RANGE)) {
        (Some(k), Some(r)) => k.iter().any(|&key| (r as u8..=(r >> 8) as u8).contains(&key)),
        _ => true,
    };
    let mut new_inst: Vec<(usize, Vec<Zone>)> = Vec::new();
    let mut inst_map = BTreeMap::new();
    let mut want_sample = BTreeMap::new();
    for (&i, keys) in &want_inst {
        let zs: Vec<Zone> = zones(&ibag, &igen, &imod, u16le(inst[i], 20) as usize, u16le(inst[i + 1], 20) as usize)
            .into_iter()
            .filter(|z| gen_of(z, GEN_SAMPLE).is_none() || plays(z, keys))
            .collect();
        // An instrument left with no samples is dropped, with the preset zones that use it.
        if !zs.iter().any(|z| gen_of(z, GEN_SAMPLE).is_some()) {
            continue;
        }
        for z in &zs {
            if let Some(s) = gen_of(z, GEN_SAMPLE) {
                let s = s as usize;
                want_sample.insert(s, ());
                // Stereo pairs travel together.
                if matches!(u16le(shdr[s], 44) & 0x7, 2 | 4) {
                    want_sample.insert(u16le(shdr[s], 42) as usize, ());
                }
            }
        }
        inst_map.insert(i, new_inst.len());
        new_inst.push((i, zs));
    }

    // Sample data: each sample copied with the 46 frames of silence the format asks for.
    let mut new_smpl = Vec::new();
    let mut sample_map = BTreeMap::new();
    let mut new_shdr: Vec<Vec<u8>> = Vec::new();
    for &s in want_sample.keys() {
        let h = shdr[s];
        let (start, end) = (u32le(h, 20) as usize, u32le(h, 24) as usize);
        let (ls, le) = (u32le(h, 28) as usize, u32le(h, 32) as usize);
        let at = new_smpl.len() / 2;
        new_smpl.extend_from_slice(&smpl[start * 2..end * 2]);
        new_smpl.extend(std::iter::repeat_n(0u8, 46 * 2));
        let mut r = h.to_vec();
        r[20..24].copy_from_slice(&(at as u32).to_le_bytes());
        r[24..28].copy_from_slice(&((at + end - start) as u32).to_le_bytes());
        r[28..32].copy_from_slice(&((at + ls - start) as u32).to_le_bytes());
        r[32..36].copy_from_slice(&((at + le - start) as u32).to_le_bytes());
        sample_map.insert(s, new_shdr.len());
        new_shdr.push(r);
    }
    for r in &mut new_shdr {
        if matches!(u16le(r, 44) & 0x7, 2 | 4) {
            let link = sample_map[&(u16le(r, 42) as usize)] as u16;
            r[42..44].copy_from_slice(&link.to_le_bytes());
        }
    }

    // Rebuild the hydra with every index remapped.
    let remap = |g: &[u8; 4], oper: u16, map: &BTreeMap<usize, usize>| -> [u8; 4] {
        let mut g = *g;
        if u16le(&g, 0) == oper {
            let to = map[&(u16le(&g, 2) as usize)] as u16;
            g[2..4].copy_from_slice(&to.to_le_bytes());
        }
        g
    };
    let (mut o_phdr, mut o_pbag, mut o_pmod, mut o_pgen) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let zone_out = |zs: &[Zone], oper, map: &BTreeMap<usize, usize>, bag: &mut Vec<u8>, gen_: &mut Vec<u8>, mod_: &mut Vec<u8>, counts: &mut (u16, u16, u16)| {
        for z in zs {
            bag.extend_from_slice(&counts.1.to_le_bytes());
            bag.extend_from_slice(&counts.2.to_le_bytes());
            for g in &z.gens {
                gen_.extend_from_slice(&remap(g, oper, map));
            }
            for m in &z.mods {
                mod_.extend_from_slice(m);
            }
            counts.0 += 1;
            counts.1 += z.gens.len() as u16;
            counts.2 += z.mods.len() as u16;
        }
    };
    let mut c = (0u16, 0u16, 0u16);
    for (p, zs) in &mut presets {
        zs.retain(|z| gen_of(z, GEN_INSTRUMENT).is_none_or(|i| inst_map.contains_key(&(i as usize))));
        let mut r = phdr[*p].to_vec();
        r[24..26].copy_from_slice(&c.0.to_le_bytes());
        o_phdr.extend_from_slice(&r);
        zone_out(zs, GEN_INSTRUMENT, &inst_map, &mut o_pbag, &mut o_pgen, &mut o_pmod, &mut c);
    }
    let mut eop = phdr[phdr.len() - 1].to_vec();
    eop[24..26].copy_from_slice(&c.0.to_le_bytes());
    o_phdr.extend_from_slice(&eop);
    o_pbag.extend_from_slice(&c.1.to_le_bytes());
    o_pbag.extend_from_slice(&c.2.to_le_bytes());
    o_pmod.extend_from_slice(&[0; 10]);
    o_pgen.extend_from_slice(&[0; 4]);

    let mut c = (0u16, 0u16, 0u16);
    let (mut o_inst, mut o_ibag, mut o_imod, mut o_igen) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, zs) in &new_inst {
        let mut r = inst[*i].to_vec();
        r[20..22].copy_from_slice(&c.0.to_le_bytes());
        o_inst.extend_from_slice(&r);
        zone_out(zs, GEN_SAMPLE, &sample_map, &mut o_ibag, &mut o_igen, &mut o_imod, &mut c);
    }
    let mut eoi = inst[inst.len() - 1].to_vec();
    eoi[20..22].copy_from_slice(&c.0.to_le_bytes());
    o_inst.extend_from_slice(&eoi);
    o_ibag.extend_from_slice(&c.1.to_le_bytes());
    o_ibag.extend_from_slice(&c.2.to_le_bytes());
    o_imod.extend_from_slice(&[0; 10]);
    o_igen.extend_from_slice(&[0; 4]);
    let mut o_shdr: Vec<u8> = new_shdr.concat();
    o_shdr.extend_from_slice(shdr[shdr.len() - 1]);

    let mut sdta = Vec::new();
    chunk(&mut sdta, b"smpl", &new_smpl);
    let mut hydra = Vec::new();
    for (id, b) in [
        (b"phdr", &o_phdr),
        (b"pbag", &o_pbag),
        (b"pmod", &o_pmod),
        (b"pgen", &o_pgen),
        (b"inst", &o_inst),
        (b"ibag", &o_ibag),
        (b"imod", &o_imod),
        (b"igen", &o_igen),
        (b"shdr", &o_shdr),
    ] {
        chunk(&mut hydra, id, b);
    }
    let mut body = b"sfbk".to_vec();
    body.extend(list_chunk(b"INFO", info));
    body.extend(list_chunk(b"sdta", &sdta));
    body.extend(list_chunk(b"pdta", &hydra));
    let mut out = Vec::new();
    chunk(&mut out, b"RIFF", &body);
    std::fs::write(&args[1], &out).expect("write the trimmed soundfont");
    println!(
        "{} presets, {} instruments, {} samples: {:.1} MB -> {:.1} MB",
        presets.len(),
        new_inst.len(),
        new_shdr.len(),
        file.len() as f64 / 1e6,
        out.len() as f64 / 1e6
    );
}

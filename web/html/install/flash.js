// Teasi map installer: everything that touches the zip, the device and
// packages.xml.  No DOM in here, so node can run it against a test folder.
'use strict';

const CHART = /^(.+)_(osm|osmarea|osmpoi|osmpoint|ta|terrain)\.v(\d{8})$/i;
const PREFIXES = ('20130125 20130212 20130213 20130807 20131010 20131020 20131026 20150215 20160505 ' +
                  '20160509 20161014 20161028 20170606 20170707 20180914 20181225 20190320 20190415 20190618').split(' ');

// ---- checksums ----------------------------------------------------------

const K = Array.from({length: 64}, (_, i) => Math.floor(Math.abs(Math.sin(i + 1)) * 2 ** 32));
const R = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];

// RFC 1321; it only ever sees 2 KB here
function md5(data) {
  const len = data.length, total = (((len + 8) >> 6) + 1) * 64;
  const buf = new Uint8Array(total);
  buf.set(data);
  buf[len] = 0x80;
  const dv = new DataView(buf.buffer);
  dv.setUint32(total - 8, (len * 8) >>> 0, true);
  dv.setUint32(total - 4, Math.floor(len / 0x20000000), true);
  let h = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
  for (let off = 0; off < total; off += 64) {
    let [a, b, c, d] = h;
    for (let i = 0; i < 64; i++) {
      let f, g;
      if (i < 16) { f = (b & c) | (~b & d); g = i; }
      else if (i < 32) { f = (d & b) | (~d & c); g = (5 * i + 1) % 16; }
      else if (i < 48) { f = b ^ c ^ d; g = (3 * i + 5) % 16; }
      else { f = c ^ (b | ~d); g = (7 * i) % 16; }
      const s = R[(i >> 4) * 4 + (i & 3)];
      const x = (a + f + K[i] + dv.getUint32(off + g * 4, true)) | 0;
      a = d; d = c; c = b;
      b = (b + ((x << s) | (x >>> (32 - s)))) | 0;
    }
    h = [h[0] + a | 0, h[1] + b | 0, h[2] + c | 0, h[3] + d | 0];
  }
  return h.map(v => [0, 8, 16, 24].map(s => ((v >>> s) & 255).toString(16).padStart(2, '0')).join('')).join('');
}

const CRC = Int32Array.from({length: 256}, (_, n) => {
  for (let k = 0; k < 8; k++) n = n & 1 ? 0xedb88320 ^ (n >>> 1) : n >>> 1;
  return n;
});
function crc32(crc, b) {
  crc = ~crc;
  for (let i = 0; i < b.length; i++) crc = CRC[(crc ^ b[i]) & 255] ^ (crc >>> 8);
  return ~crc;
}

// What packages.xml checks (documentation 5.4): size, and the MD5 of
// file[0x44:0x444] + file[-0x400:].  Fed the file in order, chunk by chunk.
class PackageSum {
  constructor() { this.size = 0; this.head = []; this.tail = new Uint8Array(0); }
  add(c) {
    const lo = Math.max(0x44, this.size), hi = Math.min(0x444, this.size + c.length);
    if (lo < hi) this.head.push(c.slice(lo - this.size, hi - this.size));
    this.tail = c.length >= 0x400 ? c.slice(-0x400) : concat(this.tail, c).slice(-0x400);
    this.size += c.length;
  }
  md5() { return md5(concat(...this.head, this.tail)); }
}

function concat(...parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

// ---- zip ----------------------------------------------------------------
// build_world.sh writes stored zips (zip -0) with the sizes in the local
// headers, so a zip can be written to the device while it downloads.

// The file list from the central directory, given the last bytes of the zip.
function centralDirectory(tail, zipSize) {
  const dv = new DataView(tail.buffer, tail.byteOffset, tail.length);
  let e = tail.length - 22;
  while (e >= 0 && dv.getUint32(e, true) !== 0x06054b50) e--;
  if (e < 0) throw new Error('not a zip file');
  const count = dv.getUint16(e + 10, true), cdOffset = dv.getUint32(e + 16, true);
  if (cdOffset === 0xffffffff) throw new Error('zip64 is not supported');
  let p = cdOffset - (zipSize - tail.length);
  if (p < 0) throw new Error('central directory out of reach');
  const files = [];
  for (let i = 0; i < count; i++) {
    if (dv.getUint32(p, true) !== 0x02014b50) throw new Error('broken central directory');
    const size = dv.getUint32(p + 24, true), nlen = dv.getUint16(p + 28, true);
    const xlen = dv.getUint16(p + 30, true), clen = dv.getUint16(p + 32, true);
    files.push({name: new TextDecoder().decode(tail.subarray(p + 46, p + 46 + nlen)), size});
    p += 46 + nlen + xlen + clen;
  }
  return files;
}

class Reader {
  constructor(stream, onBytes) {
    this.r = stream.getReader(); this.buf = new Uint8Array(0); this.done = false;
    this.onBytes = onBytes || (() => {});
  }
  async fill(n) {
    while (this.buf.length < n && !this.done) {
      const {value, done} = await this.r.read();
      if (done) this.done = true;
      else { this.buf = this.buf.length ? concat(this.buf, value) : value; }
    }
    return this.buf.length >= n;
  }
  async take(n) {
    if (!await this.fill(n)) throw new Error('the zip ends early');
    const out = this.buf.slice(0, n);
    this.buf = this.buf.subarray(n);
    this.onBytes(n);
    return out;
  }
  async piece(max) {
    if (!this.buf.length && !await this.fill(1)) throw new Error('the zip ends early');
    const k = Math.min(max, this.buf.length), out = this.buf.subarray(0, k);
    this.buf = this.buf.subarray(k);
    this.onBytes(k);
    return out;
  }
  cancel() { this.r.cancel().catch(() => {}); }
}

// The entries in order; each one has next() for its data, which the loop
// drains if the caller does not.
async function* entries(reader) {
  for (;;) {
    const sig = new DataView((await reader.take(4)).buffer).getUint32(0, true);
    if (sig === 0x02014b50 || sig === 0x06054b50) return;
    if (sig !== 0x04034b50) throw new Error('not a zip file');
    const h = await reader.take(26), v = new DataView(h.buffer);
    const flags = v.getUint16(2, true), method = v.getUint16(4, true), crc = v.getUint32(10, true);
    const size = v.getUint32(14, true), nlen = v.getUint16(22, true), xlen = v.getUint16(24, true);
    const name = new TextDecoder().decode(await reader.take(nlen));
    await reader.take(xlen);
    if (flags & 8 || size === 0xffffffff) throw new Error(name + ': sizes not in the header');
    if (method !== 0) throw new Error(name + ': compressed, expected a stored zip');
    const e = {name, size, crc, left: size};
    e.next = async () => {
      if (!e.left) return null;
      const c = await reader.piece(e.left);
      e.left -= c.length;
      return c;
    };
    yield e;
    while (await e.next()) {}
  }
}

// ---- the device ---------------------------------------------------------

async function child(dir, name, kind) {
  for await (const [n, h] of dir.entries())
    if (n.toLowerCase() === name.toLowerCase() && h.kind === kind) return h;
  return null;
}

// The picked folder may be the drive itself or BikeNav.
async function openDevice(picked) {
  const bikenav = picked.name.toLowerCase() === 'bikenav' ? picked : await child(picked, 'BikeNav', 'directory');
  if (!bikenav) throw new Error('no BikeNav folder here -- choose the Teasi drive or its BikeNav folder');
  const map = await child(bikenav, 'Map', 'directory');
  const countries = map && await child(map, 'Countries', 'directory');
  if (!countries) throw new Error('BikeNav/Map/Countries is missing');
  const dev = {bikenav, countries, serial: '', model: ''};
  const id = await child(bikenav, 'deviceid.dat', 'file');
  if (id) {
    const b = new Uint8Array(await (await id.getFile()).arrayBuffer()), v = new DataView(b.buffer);
    const n = v.getUint32(0, true);
    dev.serial = new TextDecoder().decode(b.subarray(4, 4 + n));
    const m = v.getUint32(4 + n, true);
    dev.model = new TextDecoder('utf-16le').decode(b.subarray(8 + n, 8 + n + 2 * m));
  }
  dev.supported = PREFIXES.includes(dev.serial.slice(0, 8));
  return dev;
}

// Maps on the device, by file name prefix: {prefix: {date, bytes, files: [names]}}
async function installedMaps(dev) {
  const maps = {};
  for await (const [name, h] of dev.countries.entries()) {
    const m = h.kind === 'file' && CHART.exec(name);
    if (!m) continue;
    const e = maps[m[1]] ||= {date: '', bytes: 0, files: []};
    e.files.push(name);
    e.bytes += (await h.getFile()).size;
    if (m[3] > e.date) e.date = m[3];
  }
  return maps;
}

// Leftovers of an earlier, interrupted write of the same map.
async function oldFiles(dev, prefix) {
  const out = [];
  for await (const [name, h] of dev.countries.entries()) {
    const m = CHART.exec(name.replace(/\.crswap$/i, ''));
    if (h.kind === 'file' && m && m[1].toLowerCase() === prefix.toLowerCase()) out.push(name);
  }
  return out;
}

// Point the entries of this map at the new files, drop the ones without a
// new file.  Entries of other maps and of the software stay as they are.
function patchPackages(xml, prefix, written) {
  const changed = [], removed = [];
  const out = xml.replace(/[ \t]*<file>\s*<url>([^<]*?)([^/<]+)<\/url>[\s\S]*?<\/file>[ \t]*\r?\n?/g, (blk, dir, base) => {
    const m = CHART.exec(base);
    if (!m || m[1].toLowerCase() !== prefix.toLowerCase()) return blk;
    const w = written[m[2].toLowerCase()];
    if (!w) { removed.push(base); return ''; }
    changed.push(base);
    return blk.replace(base, w.name)
              .replace(/<md5>[^<]*<\/md5>/, '<md5>' + w.md5 + '</md5>')
              .replace(/<size>[^<]*<\/size>/, '<size>' + w.size + '</size>');
  });
  return {xml: out, changed, removed};
}

async function readText(dir, name) {
  const h = await child(dir, name, 'file');
  return h ? {handle: h, text: await (await h.getFile()).text()} : null;
}

async function writeText(dir, name, text) {
  const h = await dir.getFileHandle(name, {create: true});
  const w = await h.createWritable();
  await w.write(text);
  await w.close();
}

// Remove a map and its packages.xml entries.
async function removeMap(dev, prefix, log) {
  for (const name of await oldFiles(dev, prefix)) {
    await dev.countries.removeEntry(name);
    log('deleted ' + name);
  }
  const pk = await readText(dev.bikenav, 'packages.xml');
  if (pk) {
    const p = patchPackages(pk.text, prefix, {});
    if (p.removed.length) {
      await writeText(dev.bikenav, pk.handle.name, p.xml);
      log('packages.xml: removed ' + p.removed.length + ' entries');
    }
  }
}

// Write the chart files of a zip stream into Countries.  `prefix` comes from
// the central directory; its old files are deleted first, the device has no
// room for two copies of a big country.
async function install(dev, stream, prefix, log, onBytes) {
  const pk = await readText(dev.bikenav, 'packages.xml');
  if (pk && !await child(dev.bikenav, 'packages.orig.xml', 'file')) {
    await writeText(dev.bikenav, 'packages.orig.xml', pk.text);
    log('kept the original as BikeNav/packages.orig.xml');
  }
  for (const name of await oldFiles(dev, prefix)) {
    await dev.countries.removeEntry(name);
    log('deleted ' + name);
  }
  const reader = new Reader(stream, onBytes), written = {};
  try {
    for await (const e of entries(reader)) {
      const m = CHART.exec(e.name);
      if (!m) continue;                         // README.txt
      if (m[1] !== prefix) throw new Error(e.name + ' does not belong to ' + prefix);
      const h = await dev.countries.getFileHandle(e.name, {create: true});
      const w = await h.createWritable();
      const sum = new PackageSum();
      let crc = 0, batch = [], n = 0;
      try {
        for (let c; (c = await e.next());) {
          crc = crc32(crc, c); sum.add(c);
          batch.push(c); n += c.length;
          if (n >= 4 << 20) { await w.write(concat(...batch)); batch = []; n = 0; }
        }
        if (n) await w.write(concat(...batch));
        if ((crc >>> 0) !== e.crc) throw new Error(e.name + ': checksum mismatch, the download is damaged');
        await w.close();
      } catch (err) {
        await w.abort().catch(() => {});
        await dev.countries.removeEntry(e.name).catch(() => {});
        throw err;
      }
      written[m[2].toLowerCase()] = {name: e.name, size: sum.size, md5: sum.md5()};
      log('wrote ' + e.name);
    }
  } finally {
    reader.cancel();
  }
  if (pk) {
    const p = patchPackages(pk.text, prefix, written);
    if (p.changed.length || p.removed.length) {
      await writeText(dev.bikenav, pk.handle.name, p.xml);
      log('packages.xml: ' + p.changed.length + ' entries updated, ' + p.removed.length + ' removed');
    }
  }
  return written;
}

if (typeof module !== 'undefined')
  module.exports = {md5, crc32, PackageSum, centralDirectory, Reader, entries, openDevice, installedMaps,
                    oldFiles, patchPackages, install, removeMap, CHART};

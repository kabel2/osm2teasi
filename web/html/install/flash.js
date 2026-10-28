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
// headers, so each file can be read straight out of the zip by its offset.

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
    const method = dv.getUint16(p + 10, true), crc = dv.getUint32(p + 16, true);
    const size = dv.getUint32(p + 24, true), nlen = dv.getUint16(p + 28, true);
    const xlen = dv.getUint16(p + 30, true), clen = dv.getUint16(p + 32, true);
    const offset = dv.getUint32(p + 42, true);
    const name = new TextDecoder().decode(tail.subarray(p + 46, p + 46 + nlen));
    if (method !== 0) throw new Error(name + ': compressed, expected a stored zip');
    if (size === 0xffffffff || offset === 0xffffffff) throw new Error('zip64 is not supported');
    files.push({name, size, crc, offset});
    p += 46 + nlen + xlen + clen;
  }
  return files;
}

// A broken connection, as opposed to a broken zip or device: worth trying again.
const retryable = e => e instanceof TypeError || e.retry;

// Bytes [start, end) of the zip as chunks.  `open(pos, end)` gives a stream
// from pos; when the connection breaks, `wait(attempt, error)` pauses (or
// throws to give up) and the stream is opened again where it stopped.
async function* bytes(open, start, end, wait) {
  let pos = start, attempt = 0;
  while (pos < end) {
    let r = null;
    try {
      r = (await open(pos, end)).getReader();
      for (;;) {
        const {value, done} = await r.read();
        if (done) break;
        const c = value.length > end - pos ? value.subarray(0, end - pos) : value;
        pos += c.length;
        attempt = 0;
        yield c;
        if (pos >= end) break;
      }
      if (pos < end) throw Object.assign(new Error('the download ended early'), {retry: true});
    } catch (e) {
      if (!retryable(e)) throw e;
      await wait(++attempt, e);
    } finally {
      r?.cancel().catch(() => {});
    }
  }
}

async function readAll(open, start, end, wait) {
  const parts = [];
  for await (const c of bytes(open, start, end, wait)) parts.push(c.slice());
  return concat(...parts);
}

// Where the data of a file starts: after its local header, whose extra field
// need not match the central directory's.
async function dataStart(open, f, wait) {
  const h = await readAll(open, f.offset, f.offset + 30, wait), v = new DataView(h.buffer);
  if (v.getUint32(0, true) !== 0x04034b50) throw new Error(f.name + ': no local header where expected');
  return f.offset + 30 + v.getUint16(26, true) + v.getUint16(28, true);
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

// Files in Countries that packages.xml does not list: the device ignores them,
// the TAHUNA Tool calls them unknown and offers to delete every map.  By map
// prefix ('' for files of no map): {prefix: {files: [names], broken: [names]}}.
// A broken file is empty or a .crswap, left by an install that did not finish
// (the browser creates a file under its name before writing it); the others
// were written in full, by an older installer for instance.
async function unlisted(dev) {
  const pk = await readText(dev.bikenav, 'packages.xml');
  if (!pk) return {};
  const listed = new Set([...pk.text.matchAll(FILE)].map(m => m[2]));
  const out = {};
  for await (const [name, h] of dev.countries.entries()) {
    if (h.kind !== 'file' || listed.has(name)) continue;
    const m = CHART.exec(name.replace(/\.crswap$/i, ''));
    const e = out[m ? m[1] : ''] ||= {files: [], broken: []};
    e.files.push(name);
    if (!m || name !== m[0] || !(await h.getFile()).size) e.broken.push(name);
  }
  return out;
}

// Size and checksum of a file on the device, as packages.xml carries them.
async function fileSum(h) {
  const f = await h.getFile(), part = async (a, b) => new Uint8Array(await f.slice(a, b).arrayBuffer());
  return {name: h.name, size: f.size, md5: md5(concat(await part(0x44, 0x444), await part(Math.max(0, f.size - 0x400))))};
}

// Put the files of a map that are on the device into packages.xml.
async function register(dev, prefix, log) {
  const written = {};
  for (const name of await oldFiles(dev, prefix)) {
    const m = CHART.exec(name);
    if (!m || name !== m[0]) continue;
    const l = m[2].toLowerCase();
    if (!written[l] || written[l].name < name) written[l] = await fileSum(await dev.countries.getFileHandle(name));
  }
  const pk = await readText(dev.bikenav, 'packages.xml');
  if (!pk || !Object.keys(written).length) return;
  await backup(dev, pk, log);
  const orig = await readText(dev.bikenav, 'packages.orig.xml');
  const p = patchPackages(pk.text, prefix, written, orig ? orig.text : '');
  await writeText(dev.bikenav, pk.handle.name, p.xml);
  log('packages.xml: ' + p.changed.length + ' entries updated, ' + p.added.length + ' added');
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

const OWN_ID = 9000000;   // <content> ids from here on are ours, not GPS Tuner's
// the device takes the file name from behind the url's last '/': without
// one it opens the folder itself and calls the maps not installed correctly
const DIR = 'content/';
const FILE = /[ \t]*<file>\s*<url>([^<]*?)([^/<]+)<\/url>[\s\S]*?<\/file>[ \t]*\r?\n?/g;
const mine = (base, prefix) => { const m = CHART.exec(base); return m && m[1].toLowerCase() === prefix.toLowerCase() && m; };

// Point the entries of this map at the new files, drop the ones without a
// new file.  Entries of other maps and of the software stay as they are.
// A country of the original maps keeps its <content> block when its files
// were removed; new files of that country go back into it, with the folder
// of the url taken from `orig` (packages.orig.xml) where it lists them.
function patchPackages(xml, prefix, written, orig = '') {
  const changed = [], removed = [], added = [], seen = new Set();
  let out = xml.replace(FILE, (blk, dir, base) => {
    const m = mine(base, prefix);
    if (!m) return blk;
    const w = written[m[2].toLowerCase()];
    if (!w) { removed.push(base); return ''; }
    changed.push(base); seen.add(m[2].toLowerCase());
    return blk.replace(base, w.name)
              .replace(/<md5>[^<]*<\/md5>/, '<md5>' + w.md5 + '</md5>')
              .replace(/<size>[^<]*<\/size>/, '<size>' + w.size + '</size>');
  });
  const missing = Object.keys(written).filter(l => !seen.has(l)).sort();
  if (missing.length) {
    const dirs = {};
    for (const [, dir, base] of (orig + '\n' + xml).matchAll(FILE)) {
      const m = mine(base, prefix);
      if (m) dirs[m[2].toLowerCase()] ??= dir;
    }
    const name = prefix.replace(/_/g, ' ').toLowerCase();
    out = out.replace(/([ \t]*)<content>[\s\S]*?<\/content>/g, (blk, ind) => {
      if (added.length) return blk;
      const d = /<description>([^<]*)<\/description>/.exec(blk);
      const ours = (d && d[1].trim().toLowerCase() === name) ||
                   [...blk.matchAll(FILE)].some(([, , base]) => mine(base, prefix));
      if (!ours) return blk;
      const nl = xml.includes('\r\n') ? '\r\n' : '\n', i1 = ind + ' ', i2 = ind + '  ';
      const files = missing.map(l => {
        const w = written[l];
        added.push(w.name);
        return i1 + '<file>' + nl +
               i2 + '<url>' + (dirs[l] ?? Object.values(dirs)[0] ?? DIR) + w.name + '</url>' + nl +
               i2 + '<md5>' + w.md5 + '</md5>' + nl +
               i2 + '<size>' + w.size + '</size>' + nl +
               i2 + '<destination_path>%drive_0%/Bikenav/Map/Countries</destination_path>' + nl +
               i2 + '<offset>0</offset>' + nl +
               i1 + '</file>' + nl;
      }).join('');
      return blk.replace(/[ \t]*<\/content>$/, files + ind + '</content>');
    });
    // a country the device never had gets a <content> of its own in the
    // first Map package: the TAHUNA Tool calls every file in Countries that
    // packages.xml does not list unknown and wants to delete all maps
    if (!added.length) out = out.replace(/<package>[\s\S]*?<\/package>/g, pkg => {
      const rel = /<type>Map<\/type>[\s\S]*?\n([ \t]*)<release>[\s\S]*?(\n[ \t]*<\/release>)/.exec(pkg);
      if (added.length || !rel) return pkg;
      const nl = xml.includes('\r\n') ? '\r\n' : '\n';
      const ind = (/\n([ \t]*)<content>/.exec(pkg) || [, rel[1] + ' '])[1], i1 = ind + ' ', i2 = ind + '  ';
      const ids = [...out.matchAll(/<content>\s*<id>(\d+)<\/id>/g)].map(m => +m[1]);
      const id = Math.max(OWN_ID - 1, ...ids) + 1;
      const version = (/<content>[\s\S]*?<version>([^<]*)<\/version>/.exec(pkg) ||
                       /<release>[\s\S]*?<version>([^<]*)<\/version>/.exec(pkg))[1];
      const files = missing.map(l => {
        const w = written[l];
        added.push(w.name);
        return i1 + '<file>' + nl + i2 + '<url>' + DIR + w.name + '</url>' + nl +
               i2 + '<md5>' + w.md5 + '</md5>' + nl + i2 + '<size>' + w.size + '</size>' + nl +
               i2 + '<destination_path>%drive_0%/Bikenav/Map/Countries</destination_path>' + nl +
               i2 + '<offset>0</offset>' + nl + i1 + '</file>' + nl;
      }).join('');
      const blk = nl + ind + '<content>' + nl + i1 + '<id>' + id + '</id>' + nl +
                  i1 + '<description>' + prefix.replace(/_/g, ' ') + '</description>' + nl +
                  i1 + '<version>' + version + '</version>' + nl +
                  i1 + '<icon_url>content/icon/flag/' + prefix + '.png</icon_url>' + nl +
                  files + ind + '</content>';
      const at = rel.index + rel[0].length - rel[2].length;
      return pkg.slice(0, at) + blk + pkg.slice(at);
    });
  }
  // a <content> of our own goes when its last file goes
  out = out.replace(/[ \t]*<content>\s*<id>(\d+)<\/id>(?:(?!<file>)[\s\S])*?<\/content>[ \t]*\r?\n?/g,
                    (blk, id) => +id >= OWN_ID ? '' : blk);
  return {xml: out, changed, removed, added};
}

// The first change to packages.xml keeps the original next to it.
async function backup(dev, pk, log) {
  if (pk && !await child(dev.bikenav, 'packages.orig.xml', 'file')) {
    await writeText(dev.bikenav, 'packages.orig.xml', pk.text);
    log('kept the original as BikeNav/packages.orig.xml');
  }
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
      await backup(dev, pk, log);
      await writeText(dev.bikenav, pk.handle.name, p.xml);
      log('packages.xml: removed ' + p.removed.length + ' entries');
    }
  }
}

async function removeFiles(dev, names, log) {
  for (const name of names) {
    await dev.countries.removeEntry(name);
    log('deleted ' + name);
  }
}

// Write the chart files of a zip into Countries.  `open(pos, end)` reads the
// zip from pos, `files` is its central directory.  Its old files are deleted
// first, the device has no room for two copies of a big country -- except
// those in `done`: written completely by an earlier, interrupted run of the
// same zip.  `save(done)` is called after each file so that a later run can
// go on from there; `wait` decides how long to wait out a broken connection.
async function install(dev, open, files, prefix, {log, onBytes, wait, done = {}, save = () => {}}) {
  const charts = files.filter(f => CHART.test(f.name));
  for (const f of charts)
    if (CHART.exec(f.name)[1] !== prefix) throw new Error(f.name + ' does not belong to ' + prefix);
  const keep = await finished(dev, charts, done);
  const pk = await readText(dev.bikenav, 'packages.xml');
  await backup(dev, pk, log);
  for (const name of await oldFiles(dev, prefix)) {
    if (keep[name]) continue;
    await dev.countries.removeEntry(name);
    log('deleted ' + name);
  }
  const written = {};
  for (const f of charts) {
    const layer = CHART.exec(f.name)[2].toLowerCase();
    if (keep[f.name]) {
      written[layer] = keep[f.name];
      onBytes(f.size, true);
      log('already there: ' + f.name);
      continue;
    }
    const start = await dataStart(open, f, wait);
    const h = await dev.countries.getFileHandle(f.name, {create: true});
    const w = await h.createWritable();
    const sum = new PackageSum();
    let crc = 0, batch = [], n = 0;
    try {
      for await (const c of bytes(open, start, start + f.size, wait)) {
        crc = crc32(crc, c); sum.add(c);
        batch.push(c.slice()); n += c.length;
        onBytes(c.length);
        if (n >= 4 << 20) { await w.write(concat(...batch)); batch = []; n = 0; }
      }
      if (n) await w.write(concat(...batch));
      if ((crc >>> 0) !== f.crc) throw new Error(f.name + ': checksum mismatch, the download is damaged');
      await w.close();
    } catch (err) {
      await w.abort().catch(() => {});
      await dev.countries.removeEntry(f.name).catch(() => {});
      throw err;
    }
    written[layer] = {name: f.name, size: sum.size, md5: sum.md5()};
    save(Object.fromEntries(Object.values(written).map(x => [x.name, x])));
    log('wrote ' + f.name);
  }
  if (pk) {
    const orig = await readText(dev.bikenav, 'packages.orig.xml');
    const p = patchPackages(pk.text, prefix, written, orig ? orig.text : '');
    if (p.changed.length || p.removed.length || p.added.length) {
      await writeText(dev.bikenav, pk.handle.name, p.xml);
      log('packages.xml: ' + p.changed.length + ' entries updated, ' + p.added.length + ' added, ' +
          p.removed.length + ' removed');
    }
  }
  return written;
}

// The files of `done` ({name: {name, size, md5}}) that are on the device in
// full.  The browser writes into a .crswap and renames it only on close,
// which comes after the checksum, so a file with its own name is complete.
async function finished(dev, charts, done) {
  const keep = {};
  for (const f of charts) {
    const d = done[f.name], h = d && await child(dev.countries, f.name, 'file');
    if (h && d.size === f.size && (await h.getFile()).size === f.size) keep[f.name] = d;
  }
  return keep;
}

if (typeof module !== 'undefined')
  module.exports = {md5, crc32, PackageSum, centralDirectory, bytes, openDevice, installedMaps,
                    oldFiles, patchPackages, install, finished, removeMap, unlisted, register,
                    removeFiles, CHART};

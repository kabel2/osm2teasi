// The installer against an in-memory Teasi and a connection that keeps
// breaking:  node test.js <a map zip from build_world.sh>
'use strict';
const fs = require('fs'), {execFileSync} = require('child_process');
const F = require('./html/install/flash.js');

// just enough of the File System Access API; like Chrome, a file is written
// into a .crswap and gets its name on close
class FileH {
  constructor(dir, name) { this.kind = 'file'; this.dir = dir; this.name = name; }
  async getFile() {
    const d = this.dir.files.get(this.name);
    return {size: d.length, text: async () => Buffer.from(d).toString(),
            arrayBuffer: async () => d.buffer.slice(d.byteOffset, d.byteOffset + d.length)};
  }
  async createWritable() {
    const dir = this.dir, name = this.name, swap = name + '.crswap', parts = [];
    dir.files.set(swap, new Uint8Array(0));
    return {
      write: async x => {
        if (dir.failWrite?.()) throw new DOMException('the drive is gone', 'NotFoundError');
        parts.push(Buffer.from(x));
        dir.files.set(swap, Buffer.concat(parts));
      },
      close: async () => { dir.files.delete(swap); dir.files.set(name, new Uint8Array(Buffer.concat(parts))); },
      abort: async () => { dir.files.delete(swap); },
    };
  }
}
class DirH {
  constructor(name) { this.kind = 'directory'; this.name = name; this.files = new Map(); this.dirs = new Map(); }
  async *entries() {
    yield* this.dirs;
    for (const n of [...this.files.keys()]) yield [n, new FileH(this, n)];
  }
  async getFileHandle(n, o) {
    if (!this.files.has(n)) {
      if (!o?.create) throw new DOMException(n, 'NotFoundError');
      this.files.set(n, new Uint8Array(0));
    }
    return new FileH(this, n);
  }
  async removeEntry(n) { this.files.delete(n); }
}

const path = process.argv[2];
const zip = new Uint8Array(fs.readFileSync(path));
const files = F.centralDirectory(zip.slice(-65536), zip.length);
const charts = files.filter(f => F.CHART.test(f.name));
const prefix = F.CHART.exec(charts[0].name)[1];
const quiet = {log: () => {}, onBytes: () => {}, wait: async () => {}};

// a Teasi with an old map of the same country and one of another
async function device() {
  const root = new DirH('TEASI'), bn = new DirH('BikeNav'), map = new DirH('Map'), c = new DirH('Countries');
  root.dirs.set('BikeNav', bn); bn.dirs.set('Map', map); map.dirs.set('Countries', c);
  bn.files.set('packages.xml', Buffer.from('<x>\n <file>\n  <url>content/1/' + prefix +
               '_ta.v20200101</url>\n  <md5>0</md5>\n  <size>0</size>\n </file>\n</x>\n'));
  c.files.set(prefix + '_ta.v20200101', new Uint8Array(5));
  c.files.set('Other_osm.v20200101', new Uint8Array(7));
  return {c, bn, dev: await F.openDevice(root)};
}

// a server that drops the connection every `every` bytes
let requests = 0;
const flaky = every => async (pos, end) => {
  requests++;
  let sent = 0, p = pos;
  return new ReadableStream({pull(ctl) {
    if (p >= end) return ctl.close();
    if (sent >= every) return ctl.error(new TypeError('network error'));
    const n = Math.min(1000, end - p);
    ctl.enqueue(zip.slice(p, p + n)); p += n; sent += n;
  }});
};

function check(c, label) {
  for (const f of charts) {
    const want = execFileSync('unzip', ['-p', path, f.name], {maxBuffer: 2 ** 31});
    const d = c.files.get(f.name);
    if (!d || Buffer.compare(Buffer.from(d), want)) throw new Error(label + ': ' + f.name + ' differs');
  }
  const left = [...c.files.keys()].filter(n => n.endsWith('.crswap') || n.startsWith(prefix + '_ta.v2020'));
  if (left.length) throw new Error(label + ': left over ' + left);
  if (!c.files.has('Other_osm.v20200101')) throw new Error(label + ': the other map is gone');
  console.log('ok', label);
}

(async () => {
  let t = await device();
  await F.install(t.dev, flaky(50000), files, prefix, quiet);
  check(t.c, 'connection broken every 50 KB (' + requests + ' requests)');
  const xml = Buffer.from(t.bn.files.get('packages.xml')).toString();
  const ta = charts.find(f => /_ta\./.test(f.name));
  if (ta && !xml.includes(ta.name + '</url>')) throw new Error('packages.xml not patched: ' + xml);
  console.log('ok packages.xml');

  // the drive goes away in the middle; a second run with the memo goes on
  t = await device();
  let memo = {}, writes = 0;
  t.c.failWrite = () => ++writes === 3;
  try {
    await F.install(t.dev, flaky(Infinity), files, prefix, {...quiet, save: m => memo = m});
    throw new Error('the failing drive went unnoticed');
  } catch (e) { if (e.name !== 'NotFoundError') throw e; }
  if (!Object.keys(memo).length) throw new Error('nothing remembered');
  t.c.failWrite = null;
  let skipped = 0;
  await F.install(t.dev, flaky(Infinity), files, prefix,
                  {...quiet, done: memo, onBytes: (n, s) => { if (s) skipped += n; }});
  if (!skipped) throw new Error('nothing skipped');
  check(t.c, 'went on after the drive failed, ' + Object.keys(memo).length + ' files kept');

  // remembered as done, but the file on the drive is short: written again
  const k = Object.keys(memo)[0];
  t.c.files.set(k, new Uint8Array(3));
  if ((await F.finished(t.dev, charts, memo))[k]) throw new Error('a short file was trusted');
  console.log('ok short file not trusted');

  // an original map: Remove keeps the original packages.xml, Install puts the
  // entries back into the country's <content>, with the folder from the original
  const entry = (dir, name) => '    <file>\n     <url>' + dir + name + '</url>\n     <md5>0</md5>\n' +
    '     <size>1</size>\n     <destination_path>%drive_0%/Bikenav/Map/Countries</destination_path>\n' +
    '     <offset>0</offset>\n    </file>\n';
  const block = (id, name, body) => '   <content>\n    <id>' + id + '</id>\n    <description>' + name +
    '</description>\n    <version>5130</version>\n' + body + '   </content>\n';
  const original = '<root>\n <package>\n  <type>Map</type>\n  <release>\n' +
    block(1, prefix.replace(/_/g, ' '), entry('content/1/9/', prefix + '_osm.v20210916') +
                                        entry('content/1/9/', prefix + '_osmpoint.v20210916')) +
    block(2, 'Other', entry('content/1/8/', 'Other_osm.v20200101')) +
    '  </release>\n </package>\n</root>\n';
  t = await device();
  const text = n => Buffer.from(t.bn.files.get(n)).toString();
  t.bn.files.set('packages.xml', Buffer.from(original));
  t.c.files.set(prefix + '_osm.v20210916', new Uint8Array(5));
  await F.removeMap(t.dev, prefix, () => {});
  const removed = text('packages.xml');
  if (text('packages.orig.xml') !== original) throw new Error('Remove did not keep the original');
  if (removed.includes(prefix + '_') || !removed.includes('Other_osm')) throw new Error('Remove: ' + removed);
  console.log('ok Remove keeps packages.orig.xml');
  const written = await F.install(t.dev, flaky(Infinity), files, prefix, quiet);
  const want = Object.values(written).sort((a, b) => a.name < b.name ? -1 : 1)
    .map(w => entry('content/1/9/', w.name).replace('<md5>0', '<md5>' + w.md5).replace('<size>1', '<size>' + w.size));
  if (text('packages.xml') !== original.replace(/(<description>[^<]*<\/description>\n[^\n]*\n)[\s\S]*?(   <\/content>)/,
                                                  '$1' + want.join('') + '$2'))
    throw new Error('entries not put back: ' + text('packages.xml'));
  console.log('ok Install puts ' + want.length + ' entries back');

  // the same, but packages.orig.xml was made after the Remove (older
  // installer) or is missing: the url gets the folder content/
  for (const orig of [removed, null]) {
    t = await device();
    t.bn.files.set('packages.xml', Buffer.from(removed));
    if (orig) t.bn.files.set('packages.orig.xml', Buffer.from(orig));
    await F.install(t.dev, flaky(Infinity), files, prefix, quiet);
    const x = text('packages.xml');
    if (x.split('<file>').length - 1 !== charts.length + 1 || !x.includes('<url>content/' + charts[0].name + '</url>'))
      throw new Error('entries not added: ' + x);
  }
  console.log('ok without the original folder');

  // a map the device never had gets a <content> of its own after the last
  // one of the Map package, and Remove takes it out again
  t = await device();
  const none = removed.replace(/<description>[^<]*<\/description>/, '<description>Elsewhere</description>');
  t.bn.files.set('packages.xml', Buffer.from(none));
  const w2 = await F.install(t.dev, flaky(Infinity), files, prefix, quiet);
  const own = '   <content>\n    <id>9000000</id>\n    <description>' + prefix.replace(/_/g, ' ') +
    '</description>\n    <version>5130</version>\n    <icon_url>content/icon/flag/' + prefix + '.png</icon_url>\n' +
    Object.values(w2).sort((a, b) => a.name < b.name ? -1 : 1)
      .map(w => entry('content/', w.name).replace('<md5>0', '<md5>' + w.md5).replace('<size>1', '<size>' + w.size)).join('') +
    '   </content>\n';
  if (text('packages.xml') !== none.replace('  </release>', own + '  </release>'))
    throw new Error('no <content> for a new map: ' + text('packages.xml'));
  await F.removeMap(t.dev, prefix, () => {});
  if (text('packages.xml') !== none) throw new Error('Remove left the new <content>: ' + text('packages.xml'));
  console.log('ok a new map gets its own <content>, Remove takes it out');

  // the zip changed on the server: stop, do not retry
  t = await device();
  const stale = async () => { throw Object.assign(new Error('changed'), {stale: true}); };
  try {
    await F.install(t.dev, stale, files, prefix, quiet);
    throw new Error('the changed zip went unnoticed');
  } catch (e) { if (!e.stale) throw e; }
  console.log('ok changed zip stops');
})().catch(e => { console.error('FAIL', e); process.exit(1); });

import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.symbol.*;
import java.util.*;
import java.io.*;

public class ChartHunt extends GhidraScript {
  AddressSpace space;
  Memory mem;
  PrintWriter pw, pwS, pwF;
  String[] needles = {
    "osmpoint","osmarea","osmpoi","harmonic","terrain",".pak",".idx",".tcd",
    "EarthText","mappack","sqlite","SELECT","jpeg","mmdb","db3","Map\\",
    "Countries","global.pak","pbf","uncompress","compress","inflate","deflate","zlib"
  };
  Set<String> fullMatches = new HashSet<>(Arrays.asList(
    "osm","ta","terrain","osmpoint","osmarea","osmpoi","osmworld","osmcity"));

  public void run() throws Exception {
    space = currentProgram.getAddressFactory().getDefaultAddressSpace();
    mem = currentProgram.getMemory();
    pw  = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_charts.txt"));
    pwS = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_strings_all.txt"));
    pwF = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_funcs.txt"));
    pw.println("=== CHART HUNT (no-decompile) " + currentProgram.getName() + " ===");

    for (MemoryBlock b : mem.getBlocks())
      pw.println("BLOCK " + b.getName() + " " + b.getStart() + "-" + b.getEnd() + " size=" + b.getSize());

    FunctionIterator fi = currentProgram.getFunctionManager().getFunctions(true);
    int fn = 0;
    while (fi.hasNext()) {
      Function f = fi.next();
      pwF.println(f.getEntryPoint() + " " + f.getName() + " size=" + (f.getBody().getMaxAddress().getOffset() - f.getBody().getMinAddress().getOffset() + 1));
      fn++;
    }
    pw.println("functions: " + fn);

    // per-block string scan
    List<long[]> hits = new ArrayList<>();
    for (MemoryBlock b : mem.getBlocks()) {
      long base = b.getStart().getOffset();
      byte[] buf = new byte[(int) b.getSize()];
      try { mem.getBytes(b.getStart(), buf); } catch (Exception ex) { continue; }
      int i = 0;
      while (i < buf.length - 4) {
        if (isAscii(buf, i, 4)) {
          String s = readAscii(buf, i);
          pwS.println("A " + space.getAddress(base + i) + " [" + b.getName() + "] " + s);
          if (match(s)) hits.add(new long[]{base + i, 1});
          i += Math.max(s.length(), 1);
          continue;
        }
        if (isUtf16(buf, i, 4)) {
          String s = readUtf16(buf, i);
          pwS.println("U " + space.getAddress(base + i) + " [" + b.getName() + "] " + s);
          if (match(s)) hits.add(new long[]{base + i, 2});
          i += Math.max(s.length() * 2, 2);
          continue;
        }
        i++;
      }
    }
    pw.println("string hits: " + hits.size());
    for (long[] h : hits)
      pw.println("HIT @" + space.getAddress(h[0]) + (h[1]==1?" (ascii)":" (utf16)"));

    // magic u32 scan (all blocks)
    pw.println("--- magic u32 scan ---");
    int mc = 0;
    for (MemoryBlock b : mem.getBlocks()) {
      byte[] bb = new byte[(int) b.getSize()];
      try { mem.getBytes(b.getStart(), bb); } catch (Exception ex) { continue; }
      for (int j = 0; j + 4 <= bb.length; j++) {
        int u = (bb[j]&0xff) | ((bb[j+1]&0xff)<<8) | ((bb[j+2]&0xff)<<16) | ((bb[j+3]&0xff)<<24);
        if (u == 0x1B62 || u == 0x621B || u == 0x1B5A || u == 0x5A1B) {
          pw.println("MAGIC32 @" + space.getAddress(b.getStart().getOffset()+j) + " u32=0x" + Integer.toHexString(u) + " block=" + b.getName());
          if (++mc > 400) break;
        }
      }
      if (mc > 400) break;
    }
    pw.println("magic32 hits: " + mc);

    // xrefs to hit strings -> functions
    pw.println("--- xrefs to hit strings ---");
    Set<String> done = new HashSet<>();
    List<String[]> rows = new ArrayList<>();
    for (long[] h : hits) {
      Address a = space.getAddress(h[0]);
      for (Reference r : currentProgram.getReferenceManager().getReferencesTo(a)) {
        Address from = r.getFromAddress();
        Function f = currentProgram.getFunctionManager().getFunctionContaining(from);
        String key = f == null ? from.toString() : f.getEntryPoint().toString();
        if (done.add(key)) {
          rows.add(new String[]{
            f == null ? "nofunc" : f.getEntryPoint().toString(),
            f == null ? "?" : f.getName(),
            f == null ? "?" : String.valueOf(f.getBody().getMaxAddress().getOffset() - f.getBody().getMinAddress().getOffset() + 1),
            a.toString()});
        }
      }
    }
    rows.sort((x,y) -> x[0].compareTo(y[0]));
    for (String[] r : rows)
      pw.println("XREF func=" + r[0] + " name=" + r[1] + " size=" + r[2] + " strref=" + r[3]);
    pw.println("xref funcs: " + rows.size());

    pw.flush(); pw.close(); pwS.close(); pwF.close();
    println("STAGE1 DONE: " + hits.size() + " hits, " + rows.size() + " xref funcs, " + fn + " functions");
  }

  boolean match(String s) {
    String l = s.toLowerCase();
    if (fullMatches.contains(l)) return true;
    for (String n : needles)
      if (l.contains(n.toLowerCase())) return true;
    return false;
  }
  boolean isAscii(byte[] b, int i, int min) {
    int n = 0;
    for (int j = i; j < b.length && j < i + 256; j++) {
      byte c = b[j];
      if (c == 0) break;
      if (c < 9 || c > 126) return false;
      n++;
    }
    return n >= min;
  }
  boolean isUtf16(byte[] b, int i, int min) {
    int n = 0;
    for (int j = i; j + 1 < b.length && j < i + 512; j += 2) {
      char c = (char) ((b[j] & 0xff) | ((b[j+1] & 0xff) << 8));
      if (c == 0) break;
      if (c < 9 || c > 126) return false;
      n++;
    }
    return n >= min;
  }
  String readAscii(byte[] b, int i) {
    int j = i;
    while (j < b.length && b[j] != 0 && j - i < 1024) j++;
    return new String(b, i, j - i);
  }
  String readUtf16(byte[] b, int i) {
    int j = i, n = 0;
    while (j + 1 < b.length && n < 512) {
      char c = (char) ((b[j] & 0xff) | ((b[j+1] & 0xff) << 8));
      if (c == 0) break;
      j += 2; n++;
    }
    return new String(b, i, j - i);
  }
}

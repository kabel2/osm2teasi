import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.symbol.*;
import java.util.*;
import java.io.*;

public class MapDump extends GhidraScript {
  AddressSpace space;
  Memory mem;
  PrintWriter pw;

  public void run() throws Exception {
    space = currentProgram.getAddressFactory().getDefaultAddressSpace();
    mem = currentProgram.getMemory();
    pw = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_mapdump2.txt"));
    pw.println("=== MAP DUMP (disasm, no decompiler) ===");

    // 1) raw u16 scan for 0x1B62 / 0x1B5A in all blocks
    pw.println("--- raw u16 scan 0x1b62 / 0x1b5a ---");
    int c = 0;
    for (MemoryBlock b : mem.getBlocks()) {
      byte[] bb = new byte[(int) b.getSize()];
      try { mem.getBytes(b.getStart(), bb); } catch (Exception e) { continue; }
      for (int j = 0; j + 2 <= bb.length; j++) {
        int u = (bb[j]&0xff) | ((bb[j+1]&0xff)<<8);
        if (u == 0x1B62 || u == 0x1B5A || u == 0x621B || u == 0x5A1B) {
          pw.println("U16 @" + space.getAddress(b.getStart().getOffset()+j) + " val=0x" + Integer.toHexString(u) + " block=" + b.getName());
          if (++c > 500) break;
        }
      }
      if (c > 500) break;
    }
    pw.println("u16 hits: " + c);

    // 2) xref detail for map-relevant string addresses
    long[] addrs = {
      0x4b8404, 0x4b8410, 0x4b8420, 0x4b8430, 0x4b8730, 0x4b87c0, 0x4b8828, 0x4b8834,
      0x4d0468, 0x4d04f0, 0x49f6d4, 0x49f704,
      0x50d18c, 0x50d1f8, 0x50d200, 0x50d27c, 0x50d314, 0x50d34c
    };
    pw.println("--- all xrefs to map/harmonics strings ---");
    Set<String> fnSet = new HashSet<>();
    for (long a : addrs) {
      Address addr = space.getAddress(a);
      int n = 0;
      for (Reference r : currentProgram.getReferenceManager().getReferencesTo(addr)) {
        Address from = r.getFromAddress();
        Function f = currentProgram.getFunctionManager().getFunctionContaining(from);
        pw.println("XREF str=0x" + Long.toHexString(a) + " from=" + from + " func=" + (f==null?"?":f.getEntryPoint()+" "+f.getName()) + " type=" + r.getReferenceType());
        if (f != null) fnSet.add(f.getEntryPoint().toString());
        if (++n > 40) break;
      }
    }

    // 3) disassemble all functions referencing those strings (+ nearby big funcs)
    pw.println("--- disassembly of referencing functions ---");
    for (String fs : fnSet) {
      Address ep = space.getAddress(Long.parseLong(fs.substring(2), 16));
      Function f = currentProgram.getFunctionManager().getFunctionContaining(ep);
      if (f == null) continue;
      dumpFunction(pw, f);
    }

    // 4) always dump the big map-ish functions in the 0x2xxxx-0x6xxxx range that were in stage1 xref list
    pw.println("--- additional functions (stage1 xref list sample) ---");
    String[] extra = {"000439dc","00115fa8","002363c4","0024c05c","003f0b84","003d0e70","000e2ebc","003f1554"};
    for (String e : extra) {
      Function f = currentProgram.getFunctionManager().getFunctionContaining(space.getAddress(Long.parseLong(e,16)));
      if (f != null) dumpFunction(pw, f);
    }

    pw.flush(); pw.close();
    println("MAPDUMP DONE, funcs=" + fnSet.size());
  }

  void dumpFunction(PrintWriter pw, Function f) throws Exception {
    pw.println("### FUNC " + f.getEntryPoint() + " " + f.getName()
      + " size=" + (f.getBody().getMaxAddress().getOffset() - f.getBody().getMinAddress().getOffset() + 1));
    AddressSetView body = f.getBody();
    Address a = body.getMinAddress();
    Address end = body.getMaxAddress();
    while (a.compareTo(end) <= 0) {
      Instruction ins = currentProgram.getListing().getInstructionAt(a);
      if (ins != null) {
        String s = ins.toString();
        pw.println(String.format("%08x  %s", a.getOffset(), s));
        a = ins.getMaxAddress().next();
      } else {
        a = a.next();
      }
      if (a.getOffset() > 0x600000) break;
    }
  }
}

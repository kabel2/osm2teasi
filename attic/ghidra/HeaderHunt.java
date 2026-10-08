import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class HeaderHunt extends GhidraScript {
  public void run() throws Exception {
    PrintWriter pw = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_header.txt"));
    int[] consts = {
      0x1B62, 0x1B5A, 0x159C, 0x157C, 0x140, 0x130, 0x78, 0x4C, 0x44, 0x62
    };
    Listing listing = currentProgram.getListing();
    Memory mem = currentProgram.getMemory();
    ReferenceManager rm = currentProgram.getReferenceManager();
    LinkedHashMap<String, Address> funcs = new LinkedHashMap<String, Address>();

    for (int c : consts) {
      long v = c & 0xffffffffL;
      pw.println("########## CONST 0x" + Integer.toHexString(c) + " ##########");

      pw.println("--- scalars in instructions ---");
      InstructionIterator it = listing.getInstructions(true);
      int sh = 0;
      while (it.hasNext() && !getMonitor().isCancelled()) {
        Instruction ins = it.next();
        int n = ins.getNumOperands();
        for (int i = 0; i < n; i++) {
          Object[] objs = ins.getOpObjects(i);
          for (Object o : objs) {
            if (o instanceof Scalar) {
              if ((((Scalar) o).getValue() & 0xffffffffL) == v) {
                Address a = ins.getMinAddress();
                Function f = currentProgram.getFunctionManager().getFunctionContaining(a);
                pw.println("  SCALAR " + a + " " + ins.toString()
                  + " func=" + (f == null ? "?" : f.getName() + "@" + f.getEntryPoint()));
                if (f != null) funcs.put(f.getEntryPoint().toString(), f.getEntryPoint());
                sh++;
              }
            }
          }
        }
      }
      pw.println("  scalar hits=" + sh);

      pw.println("--- 4-byte LE in memory ---");
      byte[] pat = new byte[] { (byte) (c & 0xff), (byte) ((c >> 8) & 0xff),
                                (byte) ((c >> 16) & 0xff), (byte) ((c >> 24) & 0xff) };
      Address start = mem.getMinAddress();
      int lh = 0;
      while (lh < 400 && !getMonitor().isCancelled()) {
        Address found = mem.findBytes(start, pat, null, true, getMonitor());
        if (found == null) break;
        MemoryBlock blk = mem.getBlock(found);
        ReferenceIterator ri = rm.getReferencesTo(found);
        int rc = 0;
        StringBuilder sb = new StringBuilder();
        while (ri.hasNext()) {
          Reference r = ri.next();
          rc++;
          Address from = r.getFromAddress();
          Instruction ins = listing.getInstructionAt(from);
          Function f = currentProgram.getFunctionManager().getFunctionContaining(from);
          sb.append("\n     from=" + from + " " + (ins == null ? "?" : ins.toString())
            + " func=" + (f == null ? "?" : f.getName() + "@" + f.getEntryPoint()));
          if (f != null) funcs.put(f.getEntryPoint().toString(), f.getEntryPoint());
        }
        pw.println("  LIT @" + found + " block=" + (blk == null ? "?" : blk.getName())
          + " refs=" + rc + sb.toString());
        start = found.next();
        if (start == null) break;
        lh++;
      }
      pw.println("  literal hits=" + lh);
    }

    Address extra = currentProgram.getAddressFactory().getDefaultAddressSpace().getAddress(0x000ff85cL);
    Function fe = currentProgram.getFunctionManager().getFunctionAt(extra);
    if (fe != null) funcs.put(fe.getEntryPoint().toString(), fe.getEntryPoint());

    pw.println("########## DECOMPILE " + funcs.size() + " FUNCS ##########");
    DecompInterface dci = new DecompInterface();
    if (dci.openProgram(currentProgram)) {
      int c = 0;
      for (Address a : funcs.values()) {
        if (c++ > 40) break;
        Function f = currentProgram.getFunctionManager().getFunctionContaining(a);
        if (f == null) continue;
        DecompileResults r = dci.decompileFunction(f, 60, getMonitor());
        pw.println("=== DECOMP " + f.getName() + " @" + f.getEntryPoint()
          + " size=" + f.getBody().getNumAddresses());
        if (r == null || !r.isValid()) {
          pw.println("!!! " + (r == null ? "null" : r.getErrorMessage()));
          continue;
        }
        DecompiledFunction df = r.getDecompiledFunction();
        if (df == null) { pw.println("!!! none"); continue; }
        pw.println(df.getSignature());
        pw.println(df.getC());
      }
      dci.dispose();
    } else {
      pw.println("!!! openProgram=false");
    }
    pw.flush();
    pw.close();
    println("HEADERHUNT DONE funcs=" + funcs.size());
  }
}

import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class MagicHunt extends GhidraScript {
  public void run() throws Exception {
    PrintWriter pw = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_magic.txt"));
    long[] magics = { 0x1B62L, 0x1B5AL };
    Listing listing = currentProgram.getListing();
    LinkedHashMap<String, Address> funcs = new LinkedHashMap<String, Address>();

    for (long m : magics) {
      pw.println("########## MAGIC 0x" + Long.toHexString(m) + " ##########");

      pw.println("--- A) instruction scalars ---");
      InstructionIterator it = listing.getInstructions(true);
      int hits = 0;
      while (it.hasNext() && !getMonitor().isCancelled()) {
        Instruction ins = it.next();
        int n = ins.getNumOperands();
        for (int i = 0; i < n; i++) {
          Object[] objs = ins.getOpObjects(i);
          for (Object o : objs) {
            if (o instanceof Scalar) {
              long v = ((Scalar) o).getValue();
              if (v == m) {
                Address a = ins.getMinAddress();
                Function f = currentProgram.getFunctionManager().getFunctionContaining(a);
                pw.println("SCALAR " + a + "  " + ins.toString()
                  + "   func=" + (f == null ? "?" : f.getName() + "@" + f.getEntryPoint()));
                if (f != null) funcs.put(f.getEntryPoint().toString(), f.getEntryPoint());
                hits++;
              }
            }
          }
        }
      }
      pw.println("scalar hits=" + hits);

      pw.println("--- B) literal pool 4-byte LE ---");
      Memory mem = currentProgram.getMemory();
      ReferenceManager rm = currentProgram.getReferenceManager();
      byte[] pat = new byte[] { (byte) (m & 0xff), (byte) ((m >> 8) & 0xff), 0, 0 };
      Address start = mem.getMinAddress();
      int lh = 0;
      while (lh < 500 && !getMonitor().isCancelled()) {
        Address found = mem.findBytes(start, pat, null, true, getMonitor());
        if (found == null) break;
        MemoryBlock blk = mem.getBlock(found);
        pw.println("LIT @" + found + " block=" + (blk == null ? "?" : blk.getName()));
        ReferenceIterator ri = rm.getReferencesTo(found);
        int rc = 0;
        while (ri.hasNext()) {
          Reference r = ri.next();
          rc++;
          Address from = r.getFromAddress();
          Instruction ins = listing.getInstructionAt(from);
          Function f = currentProgram.getFunctionManager().getFunctionContaining(from);
          pw.println("   from=" + from + " " + (ins == null ? "?" : ins.toString())
            + " func=" + (f == null ? "?" : f.getName() + "@" + f.getEntryPoint()));
          if (f != null) funcs.put(f.getEntryPoint().toString(), f.getEntryPoint());
        }
        pw.println("   refs=" + rc);
        start = found.next();
        if (start == null) break;
        lh++;
      }
      pw.println("literal hits=" + lh);
    }

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
    println("MAGICHUNT DONE funcs=" + funcs.size());
  }
}

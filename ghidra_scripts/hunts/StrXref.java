import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class StrXref extends GhidraScript {
  public void run() throws Exception {
    PrintWriter pw = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_strxref.txt"));
    Listing listing = currentProgram.getListing();
    FunctionManager fm = currentProgram.getFunctionManager();
    ReferenceManager rm = currentProgram.getReferenceManager();
    String[] keys = { "osm", "terrain", "countries", ".pak", "earthtext", "raster", "global." };
    LinkedHashMap<String, Address> funcs = new LinkedHashMap<String, Address>();

    DataIterator di = listing.getDefinedData(true);
    while (di.hasNext() && !getMonitor().isCancelled()) {
      Data d = di.next();
      Object v = d.getValue();
      if (!(v instanceof String)) continue;
      String s = (String) v;
      String ls = s.toLowerCase();
      boolean hit = false;
      for (String k : keys) {
        if (ls.contains(k)) { hit = true; break; }
      }
      if (!hit) continue;
      Address sa = d.getMinAddress();
      pw.println("STR \"" + s + "\" @" + sa);
      ReferenceIterator ri = rm.getReferencesTo(sa);
      int rc = 0;
      while (ri.hasNext()) {
        Reference r = ri.next();
        rc++;
        Address from = r.getFromAddress();
        Instruction ins = listing.getInstructionAt(from);
        Function f = fm.getFunctionContaining(from);
        pw.println("   xref " + from + " " + (ins == null ? "?" : ins.toString())
          + " func=" + (f == null ? "?" : f.getName() + "@" + f.getEntryPoint()));
        if (f != null) funcs.put(f.getEntryPoint().toString(), f.getEntryPoint());
      }
      if (rc == 0) pw.println("   (no xrefs)");
    }

    pw.println("########## FUNCS " + funcs.size() + " ##########");
    for (String k : funcs.keySet()) {
      Function f = fm.getFunctionContaining(funcs.get(k));
      pw.println("FUNC " + k + " " + (f == null ? "?" : f.getName()
        + " size=" + f.getBody().getNumAddresses()));
    }

    pw.println("########## DECOMPILE ##########");
    DecompInterface dci = new DecompInterface();
    if (dci.openProgram(currentProgram)) {
      int c = 0;
      for (Address a : funcs.values()) {
        if (c++ > 40) break;
        Function f = fm.getFunctionContaining(a);
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
    println("STRXREF DONE funcs=" + funcs.size());
  }
}

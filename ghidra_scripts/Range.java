import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import java.io.*;

// Usage: -postScript Range.java <outfile> <lo> <hi>   decompile all functions with entry in [lo, hi)
public class Range extends GhidraScript {
  public void run() throws Exception {
    String[] a = getScriptArgs();
    DecompInterface dci = new DecompInterface();
    dci.openProgram(currentProgram);
    PrintWriter pw = new PrintWriter(new FileWriter(a[0]));
    AddressSpace sp = currentProgram.getAddressFactory().getDefaultAddressSpace();
    Address lo = sp.getAddress(Long.parseLong(a[1], 16)), hi = sp.getAddress(Long.parseLong(a[2], 16));
    for (Function f : currentProgram.getFunctionManager().getFunctions(lo, true)) {
      if (f.getEntryPoint().compareTo(hi) >= 0) break;
      DecompileResults r = dci.decompileFunction(f, 120, getMonitor());
      pw.println("=== FUN " + f.getEntryPoint() + " " + f.getName() + " size=" + f.getBody().getNumAddresses());
      if (r == null || !r.isValid() || r.getDecompiledFunction() == null) { pw.println("!!! INVALID"); continue; }
      pw.println(r.getDecompiledFunction().getC());
      pw.flush();
    }
    pw.close();
    dci.dispose();
  }
}

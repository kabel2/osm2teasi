import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

// Usage: -postScript Dx.java <outfile> <addr|xref:addr|callers:addr> ...
//   addr        : decompile function containing addr
//   xref:addr   : list all references to addr (+ containing functions)
//   callers:addr: decompile every function referencing addr
public class Dx extends GhidraScript {
  DecompInterface dci;
  PrintWriter pw;
  Set<Function> done = new HashSet<>();

  void dec(Function f) {
    if (f == null || done.contains(f)) return;
    done.add(f);
    DecompileResults r = dci.decompileFunction(f, 120, getMonitor());
    pw.println("=== FUN " + f.getEntryPoint() + " " + f.getName() + " size=" + f.getBody().getNumAddresses());
    if (r == null || !r.isValid() || r.getDecompiledFunction() == null) {
      pw.println("!!! INVALID");
      return;
    }
    pw.println(r.getDecompiledFunction().getC());
  }

  public void run() throws Exception {
    String[] args = getScriptArgs();
    dci = new DecompInterface();
    dci.openProgram(currentProgram);
    pw = new PrintWriter(new FileWriter(args[0]));
    AddressSpace sp = currentProgram.getAddressFactory().getDefaultAddressSpace();
    FunctionManager fm = currentProgram.getFunctionManager();
    for (int i = 1; i < args.length; i++) {
      String t = args[i];
      String mode = "dec";
      if (t.contains(":")) { mode = t.substring(0, t.indexOf(':')); t = t.substring(t.indexOf(':') + 1); }
      Address a = sp.getAddress(Long.parseLong(t, 16));
      if (mode.equals("dec")) {
        Function f = fm.getFunctionContaining(a);
        if (f == null) pw.println("=== NO FUNC @" + t); else dec(f);
      } else {
        pw.println("=== XREFS to " + t);
        ReferenceIterator ri = currentProgram.getReferenceManager().getReferencesTo(a);
        List<Function> fs = new ArrayList<>();
        while (ri.hasNext()) {
          Reference r = ri.next();
          Function f = fm.getFunctionContaining(r.getFromAddress());
          pw.println("  " + r.getFromAddress() + " " + r.getReferenceType() + " in " + (f == null ? "?" : f.getEntryPoint() + " " + f.getName()));
          if (f != null && !fs.contains(f)) fs.add(f);
        }
        if (mode.equals("callers")) for (Function f : fs) dec(f);
      }
      pw.flush();
    }
    pw.close();
    dci.dispose();
  }
}

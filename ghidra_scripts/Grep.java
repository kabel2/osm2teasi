import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import java.io.*;
import java.util.regex.*;

// Usage: -postScript Grep.java <outfile> <regex> [regex2 ...]
// Decompiles every function; writes those whose C matches ALL regexes.
public class Grep extends GhidraScript {
  public void run() throws Exception {
    String[] args = getScriptArgs();
    DecompInterface dci = new DecompInterface();
    dci.openProgram(currentProgram);
    PrintWriter pw = new PrintWriter(new FileWriter(args[0]));
    Pattern[] ps = new Pattern[args.length - 1];
    for (int i = 1; i < args.length; i++) ps[i - 1] = Pattern.compile(args[i]);
    int n = 0, hit = 0;
    for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
      n++;
      DecompileResults r = dci.decompileFunction(f, 60, getMonitor());
      if (r == null || !r.isValid() || r.getDecompiledFunction() == null) continue;
      String c = r.getDecompiledFunction().getC();
      boolean ok = true;
      for (Pattern p : ps) if (!p.matcher(c).find()) { ok = false; break; }
      if (ok) {
        hit++;
        pw.println("=== FUN " + f.getEntryPoint() + " " + f.getName() + " size=" + f.getBody().getNumAddresses());
        pw.println(c);
        pw.flush();
      }
    }
    pw.println("=== DONE scanned=" + n + " hits=" + hit);
    pw.close();
    dci.dispose();
  }
}

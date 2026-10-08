import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import java.io.*;

public class DecompTest extends GhidraScript {
  public void run() throws Exception {
    DecompInterface dci = new DecompInterface();
    if (!dci.openProgram(currentProgram)) {
      println("DECOMP FAIL: openProgram=false");
      return;
    }
    println("DECOMP OK: openProgram=true");
    AddressSpace sp = currentProgram.getAddressFactory().getDefaultAddressSpace();
    PrintWriter pw = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_decomp.txt"));
    String[] targets = { "000439dc", "00115fa8", "002363c4", "0024c05c",
                         "003f0b84", "003f1554", "000e2ebc", "003d0e70" };
    int ok = 0;
    for (String t : targets) {
      Address a = sp.getAddress(Long.parseLong(t, 16));
      Function f = currentProgram.getFunctionManager().getFunctionAt(a);
      if (f == null) {
        f = currentProgram.getFunctionManager().getFunctionContaining(a);
      }
      if (f == null) {
        pw.println("=== NO FUNC @ " + t);
        continue;
      }
      DecompileResults r = dci.decompileFunction(f, 60, getMonitor());
      pw.println("=== DECOMP " + f.getName() + " @" + f.getEntryPoint()
                 + " size=" + f.getBody().getNumAddresses());
      if (r == null || !r.isValid()) {
        pw.println("!!! INVALID: " + (r == null ? "null" : r.getErrorMessage()));
        continue;
      }
      DecompiledFunction df = r.getDecompiledFunction();
      if (df == null) {
        pw.println("!!! no DecompiledFunction");
        continue;
      }
      pw.println(df.getSignature());
      pw.println(df.getC());
      ok++;
      println("decompiled " + f.getEntryPoint() + " ok");
    }
    pw.println("=== SUMMARY ok=" + ok + "/" + targets.length);
    pw.flush();
    pw.close();
    dci.dispose();
    println("DECOMP TEST DONE ok=" + ok);
  }
}

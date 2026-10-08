import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import java.io.*;
import java.util.*;

public class XorHunt extends GhidraScript {
  public void run() throws Exception {
    PrintWriter pw = new PrintWriter(new FileWriter("/home/marius/git/teasi/firmware/ghidra_xor.txt"));
    Listing listing = currentProgram.getListing();
    FunctionManager fm = currentProgram.getFunctionManager();
    HashMap<String, Integer> cnt = new HashMap<String, Integer>();
    HashMap<String, Address> faddr = new HashMap<String, Address>();
    int total = 0;
    int undef = 0;

    InstructionIterator it = listing.getInstructions(true);
    while (it.hasNext() && !getMonitor().isCancelled()) {
      Instruction ins = it.next();
      String m = ins.getMnemonicString();
      if (m == null) continue;
      if (!m.toLowerCase().startsWith("eor")) continue;
      total++;
      Address a = ins.getMinAddress();
      Function f = fm.getFunctionContaining(a);
      if (f == null) { undef++; continue; }
      String key = f.getEntryPoint().toString();
      Integer c = cnt.get(key);
      cnt.put(key, c == null ? 1 : c + 1);
      faddr.put(key, f.getEntryPoint());
    }

    pw.println("total eor=" + total + " undef=" + undef + " funcs=" + cnt.size());
    List<Map.Entry<String, Integer>> list = new ArrayList<Map.Entry<String, Integer>>(cnt.entrySet());
    Collections.sort(list, new Comparator<Map.Entry<String, Integer>>() {
      public int compare(Map.Entry<String, Integer> a, Map.Entry<String, Integer> b) {
        return b.getValue().intValue() - a.getValue().intValue();
      }
    });

    LinkedHashMap<String, Address> top = new LinkedHashMap<String, Address>();
    int n = 0;
    for (Map.Entry<String, Integer> e : list) {
      if (n++ >= 60) break;
      Function f = fm.getFunctionContaining(faddr.get(e.getKey()));
      pw.println("EOR=" + e.getValue() + " @" + e.getKey()
        + " " + (f == null ? "?" : f.getName() + " size=" + f.getBody().getNumAddresses()));
      top.put(e.getKey(), faddr.get(e.getKey()));
    }

    pw.println("########## DECOMPILE TOP ##########");
    DecompInterface dci = new DecompInterface();
    if (dci.openProgram(currentProgram)) {
      int c = 0;
      for (Address a : top.values()) {
        if (c++ > 30) break;
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
    println("XORHUNT DONE total=" + total);
  }
}

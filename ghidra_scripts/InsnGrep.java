import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import java.io.*;
import java.util.regex.*;

// Usage: -postScript InsnGrep.java <outfile> <regex>
public class InsnGrep extends GhidraScript {
  public void run() throws Exception {
    String[] a = getScriptArgs();
    PrintWriter pw = new PrintWriter(new FileWriter(a[0]));
    Pattern p = Pattern.compile(a[1]);
    FunctionManager fm = currentProgram.getFunctionManager();
    for (Instruction i : currentProgram.getListing().getInstructions(true)) {
      String s = i.toString();
      if (p.matcher(s).find()) {
        Function f = fm.getFunctionContaining(i.getAddress());
        pw.println(i.getAddress() + "  " + s + "  in " + (f == null ? "?" : f.getEntryPoint().toString()));
      }
    }
    pw.close();
  }
}

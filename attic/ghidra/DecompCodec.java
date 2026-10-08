import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import java.util.*;
import java.util.regex.*;

public class DecompCodec extends GhidraScript {
    public void run() throws Exception {
        int TIMEOUT = 30;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());

        Pattern loop = Pattern.compile("\\b(while|for|do)\\s*\\(");
        Pattern mask = Pattern.compile("&\\s*(0x[0-7]|0x1f|0x3f|0x7f|0xff|[137]\\b)");
        Pattern shl = Pattern.compile("<<");
        Pattern shr = Pattern.compile(">>");

        FunctionIterator fi = currentProgram.getFunctionManager().getFunctions(true);
        int total = 0, ok = 0, cand = 0;
        while (fi.hasNext() && !monitor.isCancelled()) {
            Function f = fi.next();
            total++;
            long sz = f.getBody().getNumAddresses();
            if (sz < 150 || sz > 4000) continue;
            DecompileResults r = dci.decompileFunction(f, TIMEOUT, monitor);
            if (!r.decompileCompleted()) continue;
            ok++;
            String c = r.getDecompiledFunction().getC();
            if (c == null) continue;
            int nloop = loop.matcher(c).find() ? 1 : 0;
            int nmask = 0;
            Matcher mm = mask.matcher(c);
            while (mm.find()) nmask++;
            boolean sh = shl.matcher(c).find() || shr.matcher(c).find();
            int nparam = 0;
            for (int k = 1; k <= 6; k++)
                if (c.contains("param_" + k)) nparam = k;
            if (nloop == 1 && sh && nmask >= 2 && nparam >= 2) {
                cand++;
                println("=== FUN " + f.getEntryPoint() + " size=" + sz
                        + " params=" + nparam + " masken=" + nmask + " ===");
                println(c);
            }
            if ((total % 1000) == 0)
                println("### progress " + total + " ok=" + ok + " cand=" + cand);
        }
        dci.dispose();
        println("### SUMMARY total=" + total + " ok=" + ok + " cand=" + cand);
    }
}

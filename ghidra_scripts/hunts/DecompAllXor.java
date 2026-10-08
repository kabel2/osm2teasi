import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import java.util.*;

public class DecompAllXor extends GhidraScript {
    public void run() throws Exception {
        int TIMEOUT = 30;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());

        FunctionIterator fi = currentProgram.getFunctionManager().getFunctions(true);
        int total = 0, ok = 0, withXor = 0, failed = 0;
        while (fi.hasNext() && !monitor.isCancelled()) {
            Function f = fi.next();
            total++;
            DecompileResults r = dci.decompileFunction(f, TIMEOUT, monitor);
            if (!r.decompileCompleted()) {
                failed++;
                continue;
            }
            ok++;
            String c = r.getDecompiledFunction().getC();
            if (c == null) continue;
            if (c.indexOf('^') >= 0) {
                withXor++;
                println("=== FUN " + f.getEntryPoint() + " size="
                        + f.getBody().getNumAddresses() + " ===");
                println(c);
            }
            if ((total % 500) == 0) {
                println("### progress " + total + " ok=" + ok
                        + " xor=" + withXor + " failed=" + failed);
            }
        }
        dci.dispose();
        println("### SUMMARY total=" + total + " ok=" + ok
                + " withXor=" + withXor + " failed=" + failed);
    }
}

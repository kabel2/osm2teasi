import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.util.*;

public class CallerHunt2 extends GhidraScript {
    long[] TARGETS = {
        0x00354610L, 0x00354c34L, 0x0031896cL, 0x00317c14L, 0x00317d74L,
        0x00317e80L,
    };
    int MAXSIZE = 8000;

    public void run() throws Exception {
        int TIMEOUT = 60;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());

        FunctionManager fm = currentProgram.getFunctionManager();
        ReferenceManager rm = currentProgram.getReferenceManager();

        Set<String> callers = new LinkedHashSet<String>();
        for (long t : TARGETS) {
            Function f = fm.getFunctionAt(toAddr(t));
            if (f == null) {
                println("### target " + Long.toHexString(t) + " fehlt");
                continue;
            }
            ReferenceIterator ci = rm.getReferencesTo(f.getEntryPoint());
            int n = 0;
            while (ci.hasNext()) {
                Reference r = ci.next();
                if (!r.getReferenceType().isCall()) continue;
                Function g = fm.getFunctionContaining(r.getFromAddress());
                if (g != null) { callers.add(g.getEntryPoint().toString()); n++; }
            }
            println("### " + f.getEntryPoint() + " size="
                    + f.getBody().getNumAddresses() + "  callers=" + n);
        }
        println("### caller gesamt: " + callers.size() + " " + callers);

        for (String c : callers) {
            Address a = toAddr(Long.parseLong(c, 16));
            Function g = fm.getFunctionContaining(a);
            if (g == null || g.getEntryPoint().compareTo(a) != 0) continue;
            long sz = g.getBody().getNumAddresses();
            println("\n=== CALLER " + c + " size=" + sz + " ===");
            if (sz > MAXSIZE) { println("  (zu gross)"); continue; }
            DecompileResults r = dci.decompileFunction(g, TIMEOUT, monitor);
            if (!r.decompileCompleted()) println("  FAILED: " + r.getErrorMessage());
            else println(r.getDecompiledFunction().getC());
        }
        dci.dispose();
        println("### done");
    }
}

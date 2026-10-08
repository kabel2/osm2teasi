import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.util.*;

public class CallerHunt extends GhidraScript {
    long[] TARGETS = {
        0x000ff85cL, 0x00100a7cL, 0x00421424L, 0x0022a6dcL, 0x0021235cL,
        0x00215138L, 0x000c294cL, 0x000c5becL, 0x000440e4L, 0x00026d74L,
        0x00027c20L, 0x0002a51cL, 0x0002a7fcL, 0x00016010L,
    };
    int MAXSIZE = 6000;

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
                println("### target " + Long.toHexString(t) + " : keine Funktion");
                continue;
            }
            println("### target " + f.getEntryPoint() + "  size="
                    + f.getBody().getNumAddresses());
            // Callees
            AddressSetView body = f.getBody();
            ReferenceIterator ri = rm.getReferenceIterator(body.getMinAddress());
            List<String> cal = new ArrayList<String>();
            while (ri.hasNext()) {
                Reference r = ri.next();
                Address to = r.getToAddress();
                if (!body.contains(to)) {
                    Function g = fm.getFunctionAt(to);
                    if (g != null && r.getReferenceType().isCall())
                        cal.add(g.getEntryPoint().toString());
                }
                if (to.compareTo(body.getMaxAddress()) > 0) break;
            }
            println("    callees: " + cal);
            // Callers
            ReferenceIterator ci = rm.getReferencesTo(f.getEntryPoint());
            while (ci.hasNext()) {
                Reference r = ci.next();
                if (!r.getReferenceType().isCall()) continue;
                Function g = fm.getFunctionContaining(r.getFromAddress());
                if (g != null) callers.add(g.getEntryPoint().toString());
            }
        }
        println("### caller gesamt: " + callers.size());
        for (String c : callers) println("    caller " + c);

        println("### decompile caller");
        for (String c : callers) {
            Address a = toAddr(Long.parseLong(c, 16));
            Function g = fm.getFunctionContaining(a);
            if (g == null || g.getEntryPoint().compareTo(a) != 0) continue;
            if (g.getBody().getNumAddresses() > MAXSIZE) {
                println("=== FUN " + c + " size=" + g.getBody().getNumAddresses()
                        + " (zu gross, uebersprungen) ===");
                continue;
            }
            println("\n=== FUN " + c + " size=" + g.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(g, TIMEOUT, monitor);
            if (!r.decompileCompleted()) {
                println("  FAILED: " + r.getErrorMessage());
            } else {
                println(r.getDecompiledFunction().getC());
            }
        }
        dci.dispose();
        println("### done");
    }
}

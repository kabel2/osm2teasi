import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.util.*;

public class ZlibHunt extends GhidraScript {
    long[] STRS = {
        0x00466d00L, 0x00466ce0L, 0x00466d68L, 0x00466d54L,
        0x00466cbcL, 0x00466d14L, 0x00466d24L, 0x00466c6cL, 0x00466c84L,
    };

    public void run() throws Exception {
        int TIMEOUT = 60;
        FunctionManager fm = currentProgram.getFunctionManager();
        ReferenceManager rm = currentProgram.getReferenceManager();

        Set<String> zlibFuns = new LinkedHashSet<String>();
        for (long s : STRS) {
            Address a = toAddr(s);
            Data dat = currentProgram.getListing().getDefinedDataAt(a);
            println("### STR " + a + " " + (dat == null ? "?" : dat.getValue()));
            ReferenceIterator ri = rm.getReferencesTo(a);
            while (ri.hasNext()) {
                Reference r = ri.next();
                Function f = fm.getFunctionContaining(r.getFromAddress());
                if (f != null) {
                    println("    ref in " + f.getEntryPoint()
                            + " size=" + f.getBody().getNumAddresses());
                    zlibFuns.add(f.getEntryPoint().toString());
                }
            }
        }
        println("### zlib-funktionen: " + zlibFuns);

        Set<String> callers = new LinkedHashSet<String>();
        for (String z : zlibFuns) {
            Address a = toAddr(Long.parseLong(z, 16));
            ReferenceIterator ri = rm.getReferencesTo(a);
            while (ri.hasNext()) {
                Reference r = ri.next();
                if (!r.getReferenceType().isCall()) continue;
                Function g = fm.getFunctionContaining(r.getFromAddress());
                if (g != null && !zlibFuns.contains(g.getEntryPoint().toString()))
                    callers.add(g.getEntryPoint().toString());
            }
        }
        println("### aufrufer von inflate: " + callers.size() + " " + callers);

        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        for (String c : callers) {
            Address a = toAddr(Long.parseLong(c, 16));
            Function g = fm.getFunctionContaining(a);
            if (g == null) continue;
            println("\n=== CALLER " + c + " size=" + g.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(g, TIMEOUT, monitor);
            if (!r.decompileCompleted()) println("  FAILED: " + r.getErrorMessage());
            else println(r.getDecompiledFunction().getC());
        }
        dci.dispose();
        println("### done");
    }
}

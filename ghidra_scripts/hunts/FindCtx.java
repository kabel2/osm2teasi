import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import java.util.*;

public class FindCtx extends GhidraScript {
    public void run() throws Exception {
        int TIMEOUT = 60;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        FunctionManager fm = currentProgram.getFunctionManager();

        // functions that use both #0x1a0 and #0xbc
        List<Function> both = new ArrayList<Function>();
        List<Function> only1a0 = new ArrayList<Function>();
        FunctionIterator fi = fm.getFunctions(true);
        while (fi.hasNext()) {
            Function g = fi.next();
            boolean a = false, b = false;
            InstructionIterator ii =
                currentProgram.getListing().getInstructions(g.getBody(), true);
            while (ii.hasNext()) {
                String s = ii.next().toString();
                if (s.contains("#0x1a0]")) a = true;
                if (s.contains("#0xbc]")) b = true;
            }
            if (a && b) both.add(g);
            else if (a) only1a0.add(g);
        }
        println("### functions with 0x1a0 AND 0xbc: " + both.size());
        for (Function g : both)
            println("--- BOTH " + g.getEntryPoint()
                    + " size=" + g.getBody().getNumAddresses());
        println("### functions with 0x1a0 (all): " + only1a0.size());
        for (Function g : only1a0)
            println("--- 1A0  " + g.getEntryPoint()
                    + " size=" + g.getBody().getNumAddresses());

        for (Function g : both) {
            if (g.getBody().getNumAddresses() > 6000) {
                println("=== FUN " + g.getEntryPoint() + " (zu gross) ===");
                continue;
            }
            println("\n=== FULL " + g.getEntryPoint()
                    + " size=" + g.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(g, TIMEOUT, monitor);
            println(r.decompileCompleted() ? r.getDecompiledFunction().getC()
                                           : "FAILED: " + r.getErrorMessage());
        }
        dci.dispose();
        println("### done");
    }
}

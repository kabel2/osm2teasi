import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import java.util.*;

public class BaseHunt extends GhidraScript {
    public void run() throws Exception {
        int TIMEOUT = 60;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        FunctionManager fm = currentProgram.getFunctionManager();

        // 1) FUN_003542c8 dekompilieren
        Function f = fm.getFunctionAt(toAddr(0x003542c8L));
        if (f != null) {
            println("=== FULL 003542c8 size=" + f.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(f, TIMEOUT, monitor);
            println(r.decompileCompleted() ? r.getDecompiledFunction().getC()
                                           : "FAILED: " + r.getErrorMessage());
        } else {
            println("003542c8 fehlt");
        }

        // 2) Funktionen mit str [r,#0xbc]  (setzen die Basis)
        List<Function> hits = new ArrayList<Function>();
        FunctionIterator fi = fm.getFunctions(true);
        while (fi.hasNext()) {
            Function g = fi.next();
            AddressSetView body = g.getBody();
            InstructionIterator ii =
                currentProgram.getListing().getInstructions(body, true);
            while (ii.hasNext()) {
                Instruction ins = ii.next();
                if (ins.getMnemonicString().startsWith("str")
                    && ins.toString().contains("#0xbc]")) {
                    hits.add(g);
                    break;
                }
            }
        }
        println("### funktionen mit str [.,#0xbc]: " + hits.size());
        for (Function g : hits)
            println("--- " + g.getEntryPoint() + " size=" + g.getBody().getNumAddresses());

        for (Function g : hits) {
            if (g.getBody().getNumAddresses() > 4000) {
                println("=== FUN " + g.getEntryPoint() + " (zu gross) ===");
                continue;
            }
            println("\n=== FUN " + g.getEntryPoint()
                    + " size=" + g.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(g, TIMEOUT, monitor);
            println(r.decompileCompleted() ? r.getDecompiledFunction().getC()
                                           : "FAILED: " + r.getErrorMessage());
        }
        dci.dispose();
        println("### done");
    }
}

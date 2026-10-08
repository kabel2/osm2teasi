import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import java.util.*;

public class TileHunt extends GhidraScript {
    public void run() throws Exception {
        int TIMEOUT = 60;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());

        FunctionManager fm = currentProgram.getFunctionManager();
        FunctionIterator fi = fm.getFunctions(true);
        List<Function> hits = new ArrayList<Function>();

        while (fi.hasNext() && !monitor.isCancelled()) {
            Function f = fi.next();
            boolean ldrh2 = false, ldr4 = false, ldrh0 = false;
            AddressSetView body = f.getBody();
            InstructionIterator ii = currentProgram.getListing().getInstructions(body, true);
            while (ii.hasNext()) {
                Instruction ins = ii.next();
                String m = ins.getMnemonicString();
                String s = ins.toString();
                if (m.equals("ldrh")) {
                    if (s.contains("#0x2]")) ldrh2 = true;
                    if (s.contains("[r") && !s.contains("#")) ldrh0 = true;
                } else if (m.equals("ldr")) {
                    if (s.contains("#0x4]")) ldr4 = true;
                }
            }
            if (ldrh2 && ldr4 && ldrh0) hits.add(f);
        }
        println("### kandidaten (ldrh[r,#2] + ldr[r,#4] + ldrh[r]): " + hits.size());
        for (Function f : hits) {
            println("--- " + f.getEntryPoint() + " size=" + f.getBody().getNumAddresses());
        }
        println("### decompile");
        for (Function f : hits) {
            println("\n=== FUN " + f.getEntryPoint() + " size="
                    + f.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(f, TIMEOUT, monitor);
            if (!r.decompileCompleted()) {
                println("  FAILED: " + r.getErrorMessage());
            } else {
                println(r.getDecompiledFunction().getC());
            }
        }
        dci.dispose();
        println("### done " + hits.size());
    }
}

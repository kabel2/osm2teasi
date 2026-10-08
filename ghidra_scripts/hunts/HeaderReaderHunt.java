import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.util.*;

public class HeaderReaderHunt extends GhidraScript {
    private boolean calls(Function function, Address target) {
        ReferenceIterator refs = currentProgram.getReferenceManager().getReferencesTo(target);
        while (refs.hasNext()) {
            Reference ref = refs.next();
            if (ref.getReferenceType().isCall()
                    && function.getBody().contains(ref.getFromAddress())) return true;
        }
        return false;
    }

    public void run() throws Exception {
        FunctionManager fm = currentProgram.getFunctionManager();
        Address reader = toAddr(0x0033279cL);
        List<Function> candidates = new ArrayList<Function>();
        FunctionIterator functions = fm.getFunctions(true);
        while (functions.hasNext()) {
            Function function = functions.next();
            if (!calls(function, reader)) continue;
            boolean chartSize = false;
            InstructionIterator instructions =
                currentProgram.getListing().getInstructions(function.getBody(), true);
            while (instructions.hasNext()) {
                String text = instructions.next().toString().toLowerCase();
                if (text.contains("#0x78") || text.contains("#0x74")
                        || text.contains("#0x100") || text.contains("#0x130")) {
                    chartSize = true;
                    break;
                }
            }
            if (chartSize) candidates.add(function);
        }
        println("### Header-reader candidates: " + candidates.size());
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        for (Function function : candidates) {
            long size = function.getBody().getNumAddresses();
            println("=== " + function.getEntryPoint() + " size=" + size + " ===");
            if (size > 7000) continue;
            DecompileResults result = dci.decompileFunction(function, 90, monitor);
            println(result.decompileCompleted() ? result.getDecompiledFunction().getC()
                                                : "FAILED: " + result.getErrorMessage());
        }
        dci.dispose();
    }
}

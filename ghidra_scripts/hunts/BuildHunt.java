import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import java.util.*;

public class BuildHunt extends GhidraScript {
    public void run() throws Exception {
        FunctionManager fm = currentProgram.getFunctionManager();
        Map<Function, Set<String>> hits = new HashMap<Function, Set<String>>();
        FunctionIterator functions = fm.getFunctions(true);
        while (functions.hasNext()) {
            Function function = functions.next();
            InstructionIterator instructions =
                currentProgram.getListing().getInstructions(function.getBody(), true);
            while (instructions.hasNext()) {
                Instruction instruction = instructions.next();
                String text = instruction.toString().toLowerCase();
                if (!text.startsWith("str")) continue;
                for (String field : new String[] {"0x17c]", "0x184]", "0x1a0]", "0x1b0]"}) {
                    if (text.contains(field)) {
                        if (!hits.containsKey(function)) hits.put(function, new TreeSet<String>());
                        hits.get(function).add(field);
                    }
                }
            }
        }

        List<Function> candidates = new ArrayList<Function>(hits.keySet());
        Collections.sort(candidates, new Comparator<Function>() {
            public int compare(Function a, Function b) {
                return a.getEntryPoint().compareTo(b.getEntryPoint());
            }
        });
        println("### Stores to osmpoint runtime-index fields: " + candidates.size());
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        for (Function function : candidates) {
            long size = function.getBody().getNumAddresses();
            println("--- " + function.getEntryPoint() + " size=" + size + " fields=" + hits.get(function));
            if (size > 10000) continue;
            DecompileResults result = dci.decompileFunction(function, 90, monitor);
            println(result.decompileCompleted() ? result.getDecompiledFunction().getC()
                                                : "FAILED: " + result.getErrorMessage());
        }
        dci.dispose();
    }
}

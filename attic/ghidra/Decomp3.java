import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;

public class Decomp3 extends GhidraScript {
    public void run() throws Exception {
        int TIMEOUT = 90;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        FunctionManager fm = currentProgram.getFunctionManager();
        long[] targets = { 0x003f2414L };
        for (long a : targets) {
            Function f = fm.getFunctionAt(toAddr(a));
            if (f == null) {
                println("=== " + Long.toHexString(a) + " fehlt ===");
                continue;
            }
            println("\n=== FULL " + f.getEntryPoint()
                    + " size=" + f.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(f, TIMEOUT, monitor);
            println(r.decompileCompleted() ? r.getDecompiledFunction().getC()
                                           : "FAILED: " + r.getErrorMessage());
            if (a == 0x002896c8L) {
                println("=== DISASM " + f.getEntryPoint() + " ===");
                Instruction instruction = getInstructionAt(f.getEntryPoint());
                while (instruction != null && f.getBody().contains(instruction.getAddress())) {
                    println(instruction.toString());
                    instruction = instruction.getNext();
                }
            }
        }
        dci.dispose();
        println("### done");
    }
}

import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;

public class DecompSome extends GhidraScript {
    long[] TARGETS = { 0x00354c34L, 0x00354610L, 0x0033279cL, 0x00330ed4L };

    public void run() throws Exception {
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        FunctionManager fm = currentProgram.getFunctionManager();
        for (long t : TARGETS) {
            Function f = fm.getFunctionAt(toAddr(t));
            if (f == null) { println("### " + Long.toHexString(t) + " fehlt"); continue; }
            println("\n=== FULL " + f.getEntryPoint() + " size="
                    + f.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(f, 90, monitor);
            if (!r.decompileCompleted()) println("  FAILED: " + r.getErrorMessage());
            else println(r.getDecompiledFunction().getC());
        }
        dci.dispose();
        println("### done");
    }
}

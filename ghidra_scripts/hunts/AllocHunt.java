import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.util.*;

public class AllocHunt extends GhidraScript {
    int MAXDEC = 40;

    public void run() throws Exception {
        int TIMEOUT = 60;
        FunctionManager fm = currentProgram.getFunctionManager();
        ReferenceManager rm = currentProgram.getReferenceManager();

        Function opNew = null, opDel = null;
        FunctionIterator fi0 = fm.getFunctions(true);
        while (fi0.hasNext()) {
            Function f = fi0.next();
            String n = f.getName();
            if (n.equals("Ordinal_1095")) opNew = f;
            if (n.equals("Ordinal_1094")) opDel = f;
        }
        if (opNew == null) { println("Ordinal_1095 not found"); return; }
        println("### operator new @ " + opNew.getEntryPoint() + "  delete @ "
                + (opDel == null ? "?" : opDel.getEntryPoint()));

        Address newEntry = opNew.getEntryPoint();
        List<Function> dyn = new ArrayList<Function>();
        int constCount = 0;

        FunctionIterator fi = fm.getFunctions(true);
        while (fi.hasNext() && !monitor.isCancelled()) {
            Function f = fi.next();
            AddressSetView body = f.getBody();
            InstructionIterator ii =
                currentProgram.getListing().getInstructions(body, true);
            List<Instruction> insList = new ArrayList<Instruction>();
            while (ii.hasNext()) insList.add(ii.next());

            boolean dynamic = false;
            for (int k = 0; k < insList.size(); k++) {
                Instruction ins = insList.get(k);
                FlowType ft = ins.getFlowType();
                if (!ft.isCall()) continue;
                Address[] flows = ins.getFlows();
                boolean hit = false;
                for (Address a : flows) if (a.equals(newEntry)) hit = true;
                if (!hit) continue;
                // Argument r0: vorherige Instruktionen untersuchen
                boolean isConst = false;
                for (int j = k - 1; j >= Math.max(0, k - 4); j--) {
                    Instruction p = insList.get(j);
                    String s = p.toString();
                    if (s.startsWith("mov r0") || s.startsWith("ldr r0,[pc")
                        || s.startsWith("ldr r0, [pc")) {
                        isConst = true;
                        break;
                    }
                    if (s.startsWith("mov r0,") || s.startsWith("ldr r0,")) break;
                }
                if (isConst) constCount++;
                else dynamic = true;
            }
            if (dynamic) dyn.add(f);
        }
        println("### functions using dynamic new: " + dyn.size()
                + "  (konstante: " + constCount + ")");
        Collections.sort(dyn, new Comparator<Function>() {
            public int compare(Function a, Function b) {
                return (int) (a.getBody().getNumAddresses() - b.getBody().getNumAddresses());
            }
        });
        for (Function f : dyn) {
            println("--- " + f.getEntryPoint() + " size=" + f.getBody().getNumAddresses());
        }

        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.setOptions(new DecompileOptions());
        println("### decompile (erste " + MAXDEC + ")");
        int n = 0;
        for (Function f : dyn) {
            if (n++ >= MAXDEC) break;
            if (f.getBody().getNumAddresses() > 3000) continue;
            println("\n=== FUN " + f.getEntryPoint() + " size="
                    + f.getBody().getNumAddresses() + " ===");
            DecompileResults r = dci.decompileFunction(f, TIMEOUT, monitor);
            if (!r.decompileCompleted()) println("  FAILED: " + r.getErrorMessage());
            else println(r.getDecompiledFunction().getC());
        }
        dci.dispose();
        println("### done");
    }
}

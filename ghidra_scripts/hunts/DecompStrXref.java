import ghidra.app.decompiler.*;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import ghidra.program.model.util.*;
import ghidra.program.model.mem.*;
import java.util.*;

public class DecompStrXref extends GhidraScript {
    static final String[] NEEDLES = {
        "Decompression error", "Decompression Error", "decompress chunk",
    };

    public void run() throws Exception {
        int TIMEOUT = 60;
        DecompInterface dci = new DecompInterface();
        dci.openProgram(currentProgram);
        dci.setSimplificationStyle("decompile");
        dci.toggleCCode(true);
        dci.toggleSyntaxTree(false);
        dci.toggleJumpLoads(true);
        dci.setOptions(new DecompileOptions());

        Listing listing = currentProgram.getListing();
        ReferenceManager rm = currentProgram.getReferenceManager();
        Memory mem = currentProgram.getMemory();

        Set<Address> strings = new HashSet<Address>();
        AddressIterator ai = mem.getAddresses(true);
        // nur .rdata/.data Bereich absuchen
        Address start = toAddr(0x00450000L);
        Address end = toAddr(0x00500000L);
        Address a = start;
        while (a.compareTo(end) < 0) {
            Data dat = listing.getDefinedDataAt(a);
            if (dat != null) {
                Object val = dat.getValue();
                if (val instanceof String) {
                    String s = (String) val;
                    for (String n : NEEDLES) {
                        if (s.contains(n)) {
                            strings.add(a);
                            break;
                        }
                    }
                }
                a = a.add(dat.getLength());
            } else {
                a = a.next();
            }
        }
        println("strings found: " + strings.size());
        for (Address s : strings) {
            Data dat = listing.getDefinedDataAt(s);
            println("STR " + s + "  " + dat.getValue());
        }

        Set<String> done = new HashSet<String>();
        for (Address s : strings) {
            ReferenceIterator ri = rm.getReferencesTo(s);
            while (ri.hasNext()) {
                Reference ref = ri.next();
                Address from = ref.getFromAddress();
                Function f = currentProgram.getFunctionManager().getFunctionContaining(from);
                if (f == null) {
                    println("  xref from " + from + " (no function)");
                    continue;
                }
                String en = f.getEntryPoint().toString();
                if (done.contains(en)) continue;
                done.add(en);
                println("\n=== FUN " + en + "  size=" + f.getBody().getNumAddresses() + " ===");
                DecompileResults r = dci.decompileFunction(f, TIMEOUT, monitor);
                if (!r.decompileCompleted()) {
                    println("  FAILED: " + r.getErrorMessage());
                } else {
                    println(r.getDecompiledFunction().getC());
                }
            }
        }
        dci.dispose();
        println("\nSUMMARY functions=" + done.size());
    }
}

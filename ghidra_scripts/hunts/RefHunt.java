import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;

public class RefHunt extends GhidraScript {
    public void run() throws Exception {
        long[] targets = { 0x002897b0L, 0x002898ecL, 0x003e6044L, 0x003e5d20L };
        ReferenceManager rm = currentProgram.getReferenceManager();
        FunctionManager fm = currentProgram.getFunctionManager();
        for (long target : targets) {
            Address address = toAddr(target);
            println("### refs to " + address);
            ReferenceIterator refs = rm.getReferencesTo(address);
            while (refs.hasNext()) {
                Reference ref = refs.next();
                Function owner = fm.getFunctionContaining(ref.getFromAddress());
                Data data = currentProgram.getListing().getDefinedDataAt(ref.getFromAddress());
                println(ref.getFromAddress() + " " + ref.getReferenceType()
                    + " owner=" + (owner == null ? "-" : owner.getEntryPoint())
                    + " data=" + (data == null ? "-" : data));
            }
        }
    }
}

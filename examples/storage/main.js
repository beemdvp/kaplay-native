// Acceptance test for debounced localStorage: writes many items in one
// frame, exits via the frame limit, and a second run verifies the final
// state survived the pre-exit flush.
const phase = globalThis.__storagePhase || "write";
if (phase === "write") {
    for (let i = 0; i < 200; i++) localStorage.setItem("key" + i, "value" + i);
    localStorage.setItem("final", "last-write-wins");
    console.log("STORAGE wrote 201 items");
} else {
    let ok = true;
    for (let i = 0; i < 200; i++) {
        if (localStorage.getItem("key" + i) !== "value" + i) { ok = false; break; }
    }
    console.log("STORAGE verify", ok, localStorage.getItem("final"));
}

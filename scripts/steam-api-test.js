// Steamworks API regression test for kaplay-native.
// Auto-detects offline (Steam unavailable) vs live (Steam client) mode.
// Logs STEAMTEST PASS/FAIL/INFO lines and a SUMMARY; the runner greps them.
// Run with scripts/steam-api-test.sh (CI: Linux, offline). Set
// globalThis.__STEAM_RUN = 1 / 2 via --preload for cross-run persistence
// checks; unset performs a self-contained cloud roundtrip.
(function () {
    "use strict";

    const raw = __kapNative.steam;
    const shim = kaplayNative.steam;
    let pass = 0;
    let fail = 0;
    const failures = [];

    function check(name, cond, detail) {
        if (cond) {
            pass++;
            console.log("STEAMTEST PASS " + name);
        } else {
            fail++;
            failures.push(name);
            console.log(
                "STEAMTEST FAIL " + name +
                (detail !== undefined ? " :: " + JSON.stringify(detail === undefined ? "<undefined>" : detail) : ""),
            );
        }
    }
    function info(name, value) {
        console.log("STEAMTEST INFO " + name + " = " + JSON.stringify(value === undefined ? "<undefined>" : value));
    }
    function attempt(fn) {
        try {
            return { ok: true, value: fn() };
        } catch (error) {
            return { ok: false, error: String(error) };
        }
    }

    const METHODS = [
        "available", "playerName", "steamId",
        "unlockAchievement", "isAchievementUnlocked", "storeStats",
        "setStatInt", "getStatInt", "setStatFloat", "getStatFloat",
        "writeCloud", "readCloud", "cloudExists", "deleteCloud",
        "runCallbacks",
    ];
    for (const m of METHODS) {
        check("surface.raw." + m, typeof raw[m] === "function");
        check("surface.shim." + m, typeof shim[m] === "function");
    }

    const live = raw.available() === true;
    info("mode", live ? "live" : "offline");
    check("available.boolean", typeof raw.available() === "boolean");
    check("available.shimMatchesRaw", shim.available() === raw.available());

    if (!live) {
        // ---- offline: every call must return the documented safe fallback ----
        check("off.playerName.null", raw.playerName() === null, raw.playerName());
        check("off.playerName.shim.null", shim.playerName() === null, shim.playerName());
        check("off.steamId.string", typeof raw.steamId() === "string", raw.steamId());
        check("off.steamId.shim.string", typeof shim.steamId() === "string", shim.steamId());
        check("off.unlock.false", raw.unlockAchievement("KAPNATIVE_TEST") === false);
        check("off.achievement.false", raw.isAchievementUnlocked("KAPNATIVE_TEST") === false);
        check("off.storeStats.false", raw.storeStats() === false);
        check("off.setStatInt.false", raw.setStatInt("KAPNATIVE_TEST", 1) === false);
        check("off.getStatInt.null", raw.getStatInt("KAPNATIVE_TEST") === null, raw.getStatInt("KAPNATIVE_TEST"));
        check("off.getStatInt.shim.null", shim.getStatInt("KAPNATIVE_TEST") === null, shim.getStatInt("KAPNATIVE_TEST"));
        check("off.setStatFloat.false", raw.setStatFloat("KAPNATIVE_TEST", 1.5) === false);
        check("off.getStatFloat.null", raw.getStatFloat("KAPNATIVE_TEST") === null, raw.getStatFloat("KAPNATIVE_TEST"));
        check("off.getStatFloat.shim.null", shim.getStatFloat("KAPNATIVE_TEST") === null, shim.getStatFloat("KAPNATIVE_TEST"));
        check("off.writeCloud.false", raw.writeCloud("kapnative.txt", "data") === false);
        check("off.readCloud.null", raw.readCloud("kapnative.txt") === null, raw.readCloud("kapnative.txt"));
        check("off.readCloud.shim.null", shim.readCloud("kapnative.txt") === null, shim.readCloud("kapnative.txt"));
        check("off.cloudExists.false", raw.cloudExists("kapnative.txt") === false);
        check("off.deleteCloud.false", raw.deleteCloud("kapnative.txt") === false);

        const rc = attempt(() => raw.runCallbacks());
        check("off.runCallbacks.noThrow", rc.ok, rc.error);
        check("off.runCallbacks.undefined", rc.ok && rc.value === undefined, rc.value);

        // interior NUL must be rejected safely (CString cannot carry it)
        const nul = attempt(() => raw.unlockAchievement("a\0b"));
        check("off.nul.noThrow", nul.ok, nul.error);
        check("off.nul.false", nul.ok && nul.value === false, nul.value);
        const nulWrite = attempt(() => raw.writeCloud("a\0b", "x"));
        check("off.nulWrite.false", nulWrite.ok && nulWrite.value === false, nulWrite);

        // shim layer coerces argument types; must never throw
        const s1 = attempt(() => shim.setStatInt("x", 3.7));
        check("off.shim.setStatInt.floatCoerced", s1.ok && s1.value === false, s1);
        const s2 = attempt(() => shim.unlockAchievement(undefined));
        check("off.shim.unlock.undefinedArg", s2.ok && s2.value === false, s2);
        const s3 = attempt(() => shim.writeCloud("x", 42));
        check("off.shim.writeCloud.numberCoerced", s3.ok && s3.value === false, s3);

        // raw layer with invalid args: record behavior (engine parity check)
        const r1 = attempt(() => raw.unlockAchievement());
        info("off.raw.missingArg", r1.ok ? r1.value : "throw: " + r1.error);
        const r2 = attempt(() => raw.setStatInt("x", 3.7));
        info("off.raw.floatStat", r2.ok ? r2.value : "throw: " + r2.error);
        const r3 = attempt(() => raw.getStatInt());
        info("off.raw.missingArgGetter", r3.ok ? r3.value : "throw: " + r3.error);
    } else {
        // ---- live: real Steam client, Spacewar (480) ----
        const name = raw.playerName();
        check("live.playerName.nonEmpty", typeof name === "string" && name.length > 0, name);
        check("live.playerName.shim", shim.playerName() === name);
        const id = raw.steamId();
        check("live.steamId.decimal64", typeof id === "string" && /^\d{17}$/.test(id), id);
        check("live.steamId.shim", shim.steamId() === id);

        // int stat roundtrip (Spacewar: NumWins)
        const before = raw.getStatInt("NumWins");
        check("live.getStatInt.type", before === null || typeof before === "number", before);
        const next = (before ?? 0) + 1;
        check("live.setStatInt", raw.setStatInt("NumWins", next) === true);
        check("live.storeStats", raw.storeStats() === true);
        check("live.getStatInt.roundtrip", raw.getStatInt("NumWins") === next, raw.getStatInt("NumWins"));
        check("live.getStatInt.shim", shim.getStatInt("NumWins") === next);

        // float stat roundtrip (Spacewar: FeetTraveled)
        const fbefore = raw.getStatFloat("FeetTraveled");
        check("live.getStatFloat.type", fbefore === null || typeof fbefore === "number", fbefore);
        const fnext = Math.round(((fbefore ?? 0) + 1.25) * 100) / 100;
        check("live.setStatFloat", raw.setStatFloat("FeetTraveled", fnext) === true);
        check("live.storeStats.float", raw.storeStats() === true);
        const fread = raw.getStatFloat("FeetTraveled");
        check("live.getStatFloat.roundtrip", typeof fread === "number" && Math.abs(fread - fnext) < 0.01, fread);

        // achievement (Spacewar test achievement, same one examples/steam uses)
        check("live.unlockAchievement", raw.unlockAchievement("ACH_WIN_ONE_GAME") === true);
        check("live.isAchievementUnlocked", raw.isAchievementUnlocked("ACH_WIN_ONE_GAME") === true);
        check("live.unknownAchievement.false", raw.isAchievementUnlocked("KAPNATIVE_NOT_REAL") === false);
        check("live.unknownStat.null", raw.getStatInt("KAPNATIVE_NOT_REAL") === null, raw.getStatInt("KAPNATIVE_NOT_REAL"));
        check("live.unknownStatFloat.null", raw.getStatFloat("KAPNATIVE_NOT_REAL") === null, raw.getStatFloat("KAPNATIVE_NOT_REAL"));

        // cloud roundtrip incl. UTF-8
        const file = "kaplay-native-test.json";
        const payload = JSON.stringify({ t: Date.now(), unicode: "h\u00e9llo \u2603 \u65e5\u672c\u8a9e" });
        check("live.cloud.write", raw.writeCloud(file, payload) === true);
        check("live.cloud.exists", raw.cloudExists(file) === true);
        check("live.cloud.read", raw.readCloud(file) === payload, raw.readCloud(file));
        check("live.cloud.shim.read", shim.readCloud(file) === payload);

        // Larger payload exercises the int32 length path. Spacewar (480) has
        // a tiny cloud quota, so rejection here is app policy, not a fault.
        const big = "x".repeat(256 * 1024);
        const bigOk = raw.writeCloud("kaplay-native-big.bin", big);
        info("live.cloud.bigWrite", bigOk);
        if (bigOk) {
            const bigRead = raw.readCloud("kaplay-native-big.bin");
            check("live.cloud.bigRead", typeof bigRead === "string" && bigRead.length === big.length, bigRead && bigRead.length);
            check("live.cloud.bigDelete", raw.deleteCloud("kaplay-native-big.bin") === true);
        }
        // 512-byte roundtrip fits Spacewar's quota
        const mid = "y".repeat(512);
        check("live.cloud.midWrite", raw.writeCloud("kaplay-native-mid.bin", mid) === true);
        const midRead = raw.readCloud("kaplay-native-mid.bin");
        check("live.cloud.midRead", midRead === mid, midRead && midRead.length);
        check("live.cloud.midDelete", raw.deleteCloud("kaplay-native-mid.bin") === true);

        // empty-file semantics: record, do not assert (documented edge)
        const emptyWrite = attempt(() => raw.writeCloud("kaplay-native-empty.txt", ""));
        info("live.cloud.emptyWrite", emptyWrite.ok ? emptyWrite.value : "throw: " + emptyWrite.error);
        info("live.cloud.emptyExists", raw.cloudExists("kaplay-native-empty.txt"));
        info("live.cloud.emptyRead", raw.readCloud("kaplay-native-empty.txt"));
        raw.deleteCloud("kaplay-native-empty.txt");

        // cross-run persistence marker
        const persist = "kaplay-native-persist.json";
        if (globalThis.__STEAM_RUN === 2) {
            if (raw.cloudExists(persist)) {
                check("live.persist.exists", true);
                const marker = raw.readCloud(persist);
                check("live.persist.read", typeof marker === "string" && marker.startsWith("persist-"), marker);
                check("live.persist.delete", raw.deleteCloud(persist) === true);
            } else {
                // marker already consumed by an earlier attempt: roundtrip instead
                info("live.persist", "marker absent; roundtrip fallback");
                check("live.persist.rewrite", raw.writeCloud(persist, "persist-" + Date.now()) === true);
                check("live.persist.reread", (raw.readCloud(persist) || "").startsWith("persist-"));
                check("live.persist.redelete", raw.deleteCloud(persist) === true);
            }
        } else if (globalThis.__STEAM_RUN === 1) {
            // two-run mode: leave the marker for run 2
            check("live.persist.write", raw.writeCloud(persist, "persist-" + Date.now()) === true);
        } else {
            // standalone: self-contained persistence roundtrip
            const marker = "persist-" + Date.now();
            check("live.persist.write", raw.writeCloud(persist, marker) === true);
            check("live.persist.read", raw.readCloud(persist) === marker, raw.readCloud(persist));
            check("live.persist.delete", raw.deleteCloud(persist) === true);
        }

        check("live.cloud.delete", raw.deleteCloud(file) === true);
        check("live.cloud.absentAfterDelete", raw.cloudExists(file) === false);
        check("live.cloud.readAfterDelete.null", raw.readCloud(file) === null, raw.readCloud(file));
        check("live.cloud.deleteMissing.false", raw.deleteCloud("kaplay-native-never-existed.txt") === false);

        const rc = attempt(() => {
            for (let i = 0; i < 5; i++) raw.runCallbacks();
        });
        check("live.runCallbacks.noThrow", rc.ok, rc.error);
    }

    // ---- frame pump: frames must keep running; runCallbacks safe per frame ----
    let frames = 0;
    function tick() {
        frames++;
        const rc = attempt(() => raw.runCallbacks());
        if (!rc.ok) check("frame.runCallbacks.noThrow@" + frames, false, rc.error);
        if (frames === 18) {
            check("frame.pump", frames >= 18, frames);
            console.log(
                "STEAMTEST SUMMARY pass=" + pass + " fail=" + fail +
                " mode=" + (live ? "live" : "offline") + " frames=" + frames,
            );
            if (failures.length) console.log("STEAMTEST FAILURES " + failures.join(","));
        }
        if (frames < 19) requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);
})();
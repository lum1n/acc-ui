// Smoke test for the generated Kotlin bindings: replays a recorded session
// through SessionView and checks the regions. Built and run by
// scripts/ffi-smoke.sh (and CI) on the JVM, with JNA on the classpath and
// -Djna.library.path pointing at libacc_ui_ffi:
//
//   java ... MainKt FIXTURE.ndjson
import io.github.lum1n.accui.AccUiException
import io.github.lum1n.accui.RunState
import io.github.lum1n.accui.SessionView
import io.github.lum1n.accui.markdownBlocks
import io.github.lum1n.accui.specVersion
import java.io.File
import kotlin.system.exitProcess

var failures = 0

fun check(ok: Boolean, what: String) {
    if (!ok) {
        println("FAIL: $what")
        failures++
    }
}

fun main(args: Array<String>) {
    if (args.size != 1) {
        println("usage: MainKt FIXTURE.ndjson")
        exitProcess(2)
    }
    SessionView().use { view ->
        view.setRoot("/work")
        var changedTurns = 0
        File(args[0]).readLines().filter { it.isNotEmpty() }.forEach { line ->
            changedTurns += view.applyJson(line).turns.size
        }
        check(changedTurns > 0, "events changed turns")

        val header = view.header()
        check(header.harness == "claude", "harness is claude, got ${header.harness}")
        check(header.title == "Inventory.py test fixes", "title, got ${header.title}")
        check(header.state == RunState.Done, "run state is done, got ${header.state}")

        val turns = view.turns()
        check(turns.isNotEmpty(), "has turns")
        check(header.turns.toInt() == turns.size, "header counts the turns")
        turns.firstOrNull()?.let {
            check(it.digest.summary.startsWith("Edited 3 files"), "digest, got ${it.digest.summary}")
        }
        check(view.files().any { it.display == "inventory.py" }, "files() has inventory.py")
        check(view.commands().isNotEmpty(), "commands() is not empty")

        try {
            view.applyJson("not json")
            check(false, "applyJson rejects bad JSON")
        } catch (_: AccUiException) {
        }

        val blocks = markdownBlocks("| a |\n|---|\n| 1 |")
        check(blocks.specJson?.contains("table") == true, "markdownBlocks makes a table")
        check(specVersion() == "acc-ui/1", "spec version")

        if (failures > 0) {
            println("$failures check(s) failed")
            exitProcess(1)
        }
        println("kotlin smoke: ok (${turns.size} turns, ${view.files().size} files)")
    }
}

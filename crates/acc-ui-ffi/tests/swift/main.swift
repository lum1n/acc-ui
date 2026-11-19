// Smoke test for the generated Swift bindings: replays a recorded session
// through SessionView and checks the regions. Built and run by
// scripts/ffi-smoke.sh (and CI) on Linux:
//
//   main FIXTURE.ndjson
import Foundation

var failures = 0
func check(_ ok: Bool, _ what: String) {
    if !ok {
        print("FAIL: \(what)")
        failures += 1
    }
}

let args = CommandLine.arguments
guard args.count == 2, let text = try? String(contentsOfFile: args[1], encoding: .utf8) else {
    print("usage: main FIXTURE.ndjson")
    exit(2)
}

let view = SessionView()
view.setRoot(root: "/work")
var changedTurns = 0
for line in text.split(separator: "\n") where !line.isEmpty {
    let changes = try view.applyJson(json: String(line))
    changedTurns += changes.turns.count
}
check(changedTurns > 0, "events changed turns")

let header = view.header()
check(header.harness == "claude", "harness is claude, got \(String(describing: header.harness))")
check(header.title == "Inventory.py test fixes", "title, got \(String(describing: header.title))")
check(header.state == .done, "run state is done, got \(header.state)")

let turns = view.turns()
check(!turns.isEmpty, "has turns")
check(Int(header.turns) == turns.count, "header counts the turns")
if let first = turns.first {
    check(first.digest.summary.hasPrefix("Edited 3 files"), "digest, got \(first.digest.summary)")
}
check(view.files().contains { $0.display == "inventory.py" }, "files() has inventory.py")
check(!view.commands().isEmpty, "commands() is not empty")

do {
    _ = try view.applyJson(json: "not json")
    check(false, "applyJson rejects bad JSON")
} catch {}

let blocks = markdownBlocks(text: "| a |\n|---|\n| 1 |")
check(blocks.specJson?.contains("table") == true, "markdownBlocks makes a table")
check(specVersion() == "acc-ui/1", "spec version")

if failures > 0 {
    print("\(failures) check(s) failed")
    exit(1)
}
print("swift smoke: ok (\(turns.count) turns, \(view.files().count) files)")

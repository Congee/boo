import Foundation

@main
struct ProtocolCodecSelfTestMain {
    static func main() {
        let sessions = WireCodec.decodeSessionList(makeSessionListPayload())
        assertEqual(sessions.count, 1, "session list count")
        assertEqual(
            sessions[0],
            ValidationSessionInfo(
                id: 7,
                name: "Tab 1",
                title: "shell",
                pwd: "/tmp",
                attached: true,
                childExited: false
            ),
            "session list decoding"
        )

        guard let state = WireCodec.decodeFullState(makeFullStatePayload()) else {
            fputs("failed to decode full-state payload\n", stderr)
            exit(1)
        }
        assertEqual(state.rows, 1, "rows decode")
        assertEqual(state.cols, 2, "cols decode")
        assertEqual(state.cursorX, 1, "cursorX decode")
        assertEqual(state.cursorY, 0, "cursorY decode")
        assertEqual(state.cursorVisible, true, "cursor visible decode")
        assertEqual(state.cells.count, 2, "cell count decode")
        assertEqual(WireCodec.screenText(from: state), "A好", "screen text decoding")

        print("iOS wire codec self-test passed")
    }
}

import Foundation

extension GSPClient {
    func listSessions() {
        listTabs()
    }

    func createSession(cols: UInt16 = 120, rows: UInt16 = 36) {
        createTab(cols: cols, rows: rows)
    }

    func destroySession(sessionId: UInt32) {
        destroyTab(tabId: sessionId)
    }

    func attach(sessionId: UInt32) {
        attach(tabId: sessionId)
    }

    func configurePreferredHostSession(sessionId: UInt32?) {
        configurePreferredHostTab(tabId: sessionId)
    }

    func clearPreferredHostSession() {
        clearPreferredHostTab()
    }

    func suppressAutomaticSessionBootstrap() {
        suppressAutomaticTabBootstrap()
    }

    func configureResumeAttachment(sessionId: UInt32, attachmentId: UInt64, resumeToken: UInt64) {
        configureResumeAttachment(tabId: sessionId, attachmentId: attachmentId, resumeToken: resumeToken)
    }
}

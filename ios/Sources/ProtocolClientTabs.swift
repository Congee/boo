import Foundation

extension GSPClient {
    @available(*, deprecated, message: "Use listTabs()")
    func listSessions() {
        listTabs()
    }

    @available(*, deprecated, message: "Use createTab(cols:rows:)")
    func createSession(cols: UInt16 = 120, rows: UInt16 = 36) {
        createTab(cols: cols, rows: rows)
    }

    @available(*, deprecated, message: "Use destroyTab(tabId:)")
    func destroySession(sessionId: UInt32) {
        destroyTab(tabId: sessionId)
    }

    @available(*, deprecated, message: "Use attach(tabId:)")
    func attach(sessionId: UInt32) {
        attach(tabId: sessionId)
    }

    @available(*, deprecated, message: "Use configurePreferredHostTab(tabId:)")
    func configurePreferredHostSession(sessionId: UInt32?) {
        configurePreferredHostTab(tabId: sessionId)
    }

    @available(*, deprecated, message: "Use clearPreferredHostTab()")
    func clearPreferredHostSession() {
        clearPreferredHostTab()
    }

    @available(*, deprecated, message: "Use suppressAutomaticTabBootstrap()")
    func suppressAutomaticSessionBootstrap() {
        suppressAutomaticTabBootstrap()
    }

    @available(*, deprecated, message: "Use configureResumeAttachment(tabId:attachmentId:resumeToken:)")
    func configureResumeAttachment(sessionId: UInt32, attachmentId: UInt64, resumeToken: UInt64) {
        configureResumeAttachment(tabId: sessionId, attachmentId: attachmentId, resumeToken: resumeToken)
    }
}

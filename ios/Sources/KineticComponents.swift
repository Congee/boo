import SwiftUI
import UIKit

enum BooTab: String, CaseIterable {
    case sessions
    case history
    case settings

    var icon: String {
        switch self {
        case .sessions: return "terminal"
        case .history: return "clock.arrow.circlepath"
        case .settings: return "gearshape"
        }
    }
}

enum RemoteTerminalGestureAction {
    case pageUp
    case pageDown
    case arrowLeft
    case arrowRight
}

struct KineticTopBar: View {
    let title: String
    let subtitle: String?
    var compact = false
    var showBrand = true
    var trailingSystemImage: String? = nil
    var trailingAccessibilityLabel: String? = nil
    var trailingAction: (() -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: KineticSpacing.xs) {
            HStack {
                if showBrand {
                    HStack(spacing: KineticSpacing.sm) {
                        Image("boo-logo-mark")
                            .resizable()
                            .interpolation(.high)
                            .frame(width: 28, height: 28)
                            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .stroke(KineticColor.primary.opacity(0.35), lineWidth: 1)
                            )
                            .shadow(color: .black.opacity(0.18), radius: 10, x: 0, y: 3)
                            .accessibilityHidden(true)
                        Text("boo")
                            .font(.system(size: 20, weight: .black, design: .monospaced))
                            .foregroundStyle(KineticColor.primary)
                    }
                }
                Spacer()
                if let trailingSystemImage, let trailingAction {
                    Button(action: trailingAction) {
                        Image(systemName: trailingSystemImage)
                            .font(.system(size: compact ? 18 : 20, weight: .semibold))
                            .foregroundStyle(KineticColor.secondary)
                            .frame(width: compact ? 36 : 40, height: compact ? 36 : 40)
                            .background(KineticColor.surfaceContainerHighest)
                            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                    }
                    .accessibilityLabel(trailingAccessibilityLabel ?? trailingSystemImage)
                }
            }
            Text(title)
                .font(compact ? .system(size: 18, weight: .semibold) : KineticFont.headline)
                .foregroundStyle(KineticColor.onSurface)
                .accessibilityIdentifier("screen-title")
            if let subtitle {
                Text(subtitle)
                    .font(KineticFont.caption)
                    .foregroundStyle(KineticColor.onSurfaceVariant)
                    .accessibilityIdentifier("screen-subtitle")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, KineticSpacing.md)
        .padding(.top, compact ? KineticSpacing.sm : KineticSpacing.lg)
        .padding(.bottom, compact ? KineticSpacing.xs : KineticSpacing.md)
    }
}

struct KineticTabBar: View {
    @Binding var selectedTab: BooTab

    var body: some View {
        HStack(spacing: KineticSpacing.xxl) {
            ForEach(BooTab.allCases, id: \.self) { tab in
                Button {
                    selectedTab = tab
                } label: {
                    Image(systemName: tab.icon)
                        .font(.system(size: 22))
                        .foregroundStyle(selectedTab == tab ? KineticColor.primary : KineticColor.onSurfaceVariant)
                        .frame(width: 48, height: 48)
                        .background(selectedTab == tab ? KineticColor.primary.opacity(0.15) : .clear)
                        .clipShape(Circle())
                }
                .accessibilityIdentifier("tab-\(tab.rawValue)")
            }
        }
        .padding(.vertical, KineticSpacing.sm)
        .padding(.horizontal, KineticSpacing.xl)
        .background(KineticColor.surfaceContainerHigh.opacity(0.9))
        .clipShape(Capsule())
    }
}

struct KineticInputField: View {
    let placeholder: String
    @Binding var text: String
    var keyboardType: UIKeyboardType = .default
    var secure = false
    var accessibilityIdentifier: String? = nil

    var body: some View {
        Group {
            if secure {
                SecureUIKitTextField(
                    placeholder: placeholder,
                    text: $text,
                    keyboardType: keyboardType,
                    accessibilityIdentifier: accessibilityIdentifier ?? placeholder
                )
            } else {
                TextField(placeholder, text: $text)
                    .keyboardType(keyboardType)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                    .accessibilityIdentifier(accessibilityIdentifier ?? placeholder)
            }
        }
        .font(KineticFont.monoInput)
        .foregroundStyle(KineticColor.secondary)
        .padding(KineticSpacing.md)
        .background(KineticColor.surfaceContainerLowest)
        .clipShape(RoundedRectangle(cornerRadius: KineticRadius.container))
    }
}

private struct SecureUIKitTextField: UIViewRepresentable {
    let placeholder: String
    @Binding var text: String
    let keyboardType: UIKeyboardType
    let accessibilityIdentifier: String

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    func makeUIView(context: Context) -> UITextField {
        let field = UITextField(frame: .zero)
        field.delegate = context.coordinator
        field.addTarget(context.coordinator, action: #selector(Coordinator.editingChanged(_:)), for: .editingChanged)
        field.placeholder = placeholder
        field.isSecureTextEntry = true
        field.keyboardType = keyboardType
        field.autocorrectionType = .no
        field.autocapitalizationType = .none
        field.smartInsertDeleteType = .no
        field.smartDashesType = .no
        field.smartQuotesType = .no
        field.textContentType = .password
        field.borderStyle = .none
        field.backgroundColor = .clear
        field.textColor = UIColor(KineticColor.secondary)
        field.font = UIFont.monospacedSystemFont(ofSize: 16, weight: .regular)
        field.accessibilityIdentifier = accessibilityIdentifier
        return field
    }

    func updateUIView(_ uiView: UITextField, context: Context) {
        if uiView.text != text {
            uiView.text = text
        }
        uiView.placeholder = placeholder
        uiView.keyboardType = keyboardType
        uiView.accessibilityIdentifier = accessibilityIdentifier
    }

    final class Coordinator: NSObject, UITextFieldDelegate {
        private var text: Binding<String>

        init(text: Binding<String>) {
            self.text = text
        }

        @objc func editingChanged(_ sender: UITextField) {
            text.wrappedValue = sender.text ?? ""
        }

        func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
            guard let current = textField.text,
                  let stringRange = Range(range, in: current) else {
                return true
            }
            text.wrappedValue = current.replacingCharacters(in: stringRange, with: string)
            return true
        }
    }
}

struct KineticSectionLabel: View {
    let text: String

    var body: some View {
        Text(text.uppercased())
            .font(KineticFont.sectionLabel)
            .tracking(2)
            .foregroundStyle(KineticColor.onSurfaceVariant)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct KineticCardRow: View {
    let icon: String
    let title: String
    let subtitle: String
    var trailingText: String? = nil
    var trailingAccessibilityIdentifier: String? = nil
    var subtitleAccessoryText: String? = nil
    var subtitleAccessoryColor: Color = KineticColor.onSurfaceVariant
    var onTap: (() -> Void)? = nil
    var accessibilityIdentifier: String? = nil

    var body: some View {
        Group {
            if let onTap {
                Button(action: onTap) {
                    rowContent
                }
                .buttonStyle(.plain)
            } else {
                rowContent
            }
        }
        .accessibilityIdentifier(accessibilityIdentifier ?? title)
    }

    private var rowContent: some View {
        ZStack(alignment: .trailing) {
            HStack(spacing: KineticSpacing.md) {
                Image(systemName: icon)
                    .font(.system(size: 20))
                    .foregroundStyle(KineticColor.primary)
                    .frame(width: 40, height: 40)
                    .background(KineticColor.surfaceContainerHighest)
                    .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                VStack(alignment: .leading, spacing: KineticSpacing.xs) {
                    HStack(alignment: .firstTextBaseline, spacing: KineticSpacing.sm) {
                        Text(title)
                            .font(KineticFont.bodySmall)
                            .fontWeight(.bold)
                            .foregroundStyle(KineticColor.onSurface)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        if let trailingText, !trailingText.isEmpty {
                            Text(trailingText)
                                .font(KineticFont.caption)
                                .fontWeight(.semibold)
                                .foregroundStyle(KineticColor.primary)
                                .fixedSize(horizontal: true, vertical: true)
                                .accessibilityIdentifier(trailingAccessibilityIdentifier ?? "\(title)-metric")
                        }
                    }
                    HStack(alignment: .firstTextBaseline, spacing: KineticSpacing.sm) {
                        Text(subtitle)
                            .font(KineticFont.caption)
                            .foregroundStyle(KineticColor.onSurfaceVariant)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        if let subtitleAccessoryText, !subtitleAccessoryText.isEmpty {
                            Text(subtitleAccessoryText)
                                .font(KineticFont.caption)
                                .fontWeight(.semibold)
                                .foregroundStyle(subtitleAccessoryColor)
                                .fixedSize(horizontal: true, vertical: true)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if onTap != nil {
                    Image(systemName: "chevron.right")
                        .foregroundStyle(KineticColor.onSurfaceVariant)
                        .fixedSize()
                }
            }
        }
        .padding(KineticSpacing.md)
        .containerCard()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(accessibilityIdentifier ?? title)
    }
}

struct RemoteTerminalView: View {
    @ObservedObject var screen: ScreenState
    var onResize: ((UInt16, UInt16) -> Void)?
    var onGestureAction: ((RemoteTerminalGestureAction) -> Void)?

    private let font = Font.system(size: 14, design: .monospaced)
    private let cellWidth: CGFloat = 8.4
    private let cellHeight: CGFloat = 17
    private let gestureThreshold: CGFloat = 28

    var body: some View {
        GeometryReader { geo in
            Canvas { context, size in
                let cols = Int(screen.cols)
                let rows = Int(screen.rows)
                guard cols > 0, rows > 0, screen.cells.count == cols * rows else { return }
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.black))
                for row in 0..<rows {
                    for col in 0..<cols {
                        let cell = screen.getCell(col: col, row: row)
                        let x = CGFloat(col) * cellWidth
                        let y = CGFloat(row) * cellHeight
                        if cell.hasBg {
                            let rect = CGRect(x: x, y: y, width: cellWidth, height: cellHeight)
                            context.fill(
                                Path(rect),
                                with: .color(Color(
                                    red: Double(cell.bg_r) / 255,
                                    green: Double(cell.bg_g) / 255,
                                    blue: Double(cell.bg_b) / 255
                                ))
                            )
                        }
                        guard cell.codepoint > 0x20, let scalar = Unicode.Scalar(cell.codepoint) else { continue }
                        let color: Color = cell.hasFg
                            ? Color(red: Double(cell.fg_r)/255, green: Double(cell.fg_g)/255, blue: Double(cell.fg_b)/255)
                            : .white
                        var text = Text(String(Character(scalar))).font(font).foregroundColor(color)
                        if cell.isBold { text = text.bold() }
                        if cell.isItalic { text = text.italic() }
                        context.draw(context.resolve(text), at: CGPoint(x: x, y: y), anchor: .topLeading)
                    }
                }
                if screen.cursorVisible {
                    let rect = CGRect(
                        x: CGFloat(screen.cursorX) * cellWidth,
                        y: CGFloat(screen.cursorY) * cellHeight,
                        width: cellWidth,
                        height: cellHeight
                    )
                    context.fill(Path(rect), with: .color(.white.opacity(0.45)))
                }
            }
            .onAppear {
                onResize?(max(1, UInt16(geo.size.width / cellWidth)), max(1, UInt16(geo.size.height / cellHeight)))
            }
            .onChange(of: geo.size) { _, newSize in
                onResize?(max(1, UInt16(newSize.width / cellWidth)), max(1, UInt16(newSize.height / cellHeight)))
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: gestureThreshold)
                    .onEnded { drag in
                        guard let onGestureAction else { return }
                        let dx = drag.translation.width
                        let dy = drag.translation.height
                        if abs(dy) >= abs(dx) {
                            if dy <= -gestureThreshold {
                                onGestureAction(.pageUp)
                            } else if dy >= gestureThreshold {
                                onGestureAction(.pageDown)
                            }
                        }
                    }
            )
        }
        .background(.black)
    }
}

struct TerminalKeyboardBridge: UIViewRepresentable {
    @Binding var isFocused: Bool
    let onText: (String) -> Void
    let onBackspace: () -> Void
    let accessoryState: TerminalKeyboardAccessoryState

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> TerminalProxyTextView {
        let textView = TerminalProxyTextView(frame: .zero, textContainer: nil)
        textView.delegate = context.coordinator
        textView.onText = onText
        textView.onBackspace = onBackspace
        textView.autocorrectionType = .no
        textView.autocapitalizationType = .none
        textView.spellCheckingType = .no
        textView.smartQuotesType = .no
        textView.smartDashesType = .no
        textView.smartInsertDeleteType = .no
        textView.returnKeyType = .default
        textView.tintColor = .clear
        textView.textColor = .clear
        textView.backgroundColor = .clear
        textView.keyboardAppearance = .dark
        textView.translatesAutoresizingMaskIntoConstraints = false
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        textView.isScrollEnabled = false
        textView.accessibilityIdentifier = "terminal-text-proxy"
        textView.updateAccessory(state: accessoryState)
        return textView
    }

    func updateUIView(_ uiView: TerminalProxyTextView, context: Context) {
        uiView.onText = onText
        uiView.onBackspace = onBackspace
        uiView.updateAccessory(state: accessoryState)
        uiView.setFocus(isFocused)
        if !uiView.isFirstResponder {
            uiView.text = ""
        }
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        let parent: TerminalKeyboardBridge

        init(parent: TerminalKeyboardBridge) {
            self.parent = parent
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            if !parent.isFocused {
                DispatchQueue.main.async {
                    self.parent.isFocused = true
                }
            }
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            if parent.isFocused {
                DispatchQueue.main.async {
                    self.parent.isFocused = false
                }
            }
        }
    }
}

struct TerminalKeyboardAccessoryState {
    var ctrlActive: Bool
    var altActive: Bool
    var metaActive: Bool
    let onDismissKeyboard: () -> Void
    let onInsertText: (String) -> Void
    let onEscape: () -> Void
    let onToggleCtrl: () -> Void
    let onToggleAlt: () -> Void
    let onToggleMeta: () -> Void
    let onTab: () -> Void
    let onArrowUp: () -> Void
    let onArrowDown: () -> Void
    let onArrowLeft: () -> Void
    let onArrowRight: () -> Void
    let onPageUp: () -> Void
    let onPageDown: () -> Void
    let onHome: () -> Void
    let onEnd: () -> Void
}

final class TerminalProxyTextView: UITextView {
    var onText: ((String) -> Void)?
    var onBackspace: (() -> Void)?
    private var desiredFocus = false
    private var accessoryState: TerminalKeyboardAccessoryState?

    override var canBecomeFirstResponder: Bool { true }

    override init(frame: CGRect, textContainer: NSTextContainer?) {
        super.init(frame: frame, textContainer: textContainer)
        commonInit()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        commonInit()
    }

    private func commonInit() {
        setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        setContentCompressionResistancePriority(.defaultLow, for: .vertical)
        inputAssistantItem.allowsHidingShortcuts = false
    }

    func updateAccessory(state: TerminalKeyboardAccessoryState) {
        accessoryState = state
        applyAssistantBarItems()
    }

    func setFocus(_ focused: Bool) {
        desiredFocus = focused
        guard window != nil else { return }
        if focused, !isFirstResponder {
            DispatchQueue.main.async { [weak self] in
                guard let self, self.desiredFocus, self.window != nil, !self.isFirstResponder else { return }
                _ = self.becomeFirstResponder()
            }
        } else if !focused, isFirstResponder {
            DispatchQueue.main.async { [weak self] in
                guard let self, !self.desiredFocus, self.isFirstResponder else { return }
                _ = self.resignFirstResponder()
            }
        }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        setFocus(desiredFocus)
        applyAssistantBarItems()
    }

    override func becomeFirstResponder() -> Bool {
        let becameFirstResponder = super.becomeFirstResponder()
        if becameFirstResponder {
            selectedTextRange = textRange(from: beginningOfDocument, to: beginningOfDocument)
        }
        return becameFirstResponder
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        _ = becomeFirstResponder()
        super.touchesEnded(touches, with: event)
    }

    override func accessibilityActivate() -> Bool {
        becomeFirstResponder()
        return true
    }

    override func deleteBackward() {
        onBackspace?()
        text = ""
    }

    override func insertText(_ text: String) {
        guard !text.isEmpty else { return }
        onText?(text.replacingOccurrences(of: "\n", with: "\r"))
        self.text = ""
    }

    override func caretRect(for position: UITextPosition) -> CGRect {
        .zero
    }

    override func selectionRects(for range: UITextRange) -> [UITextSelectionRect] {
        []
    }
    
    private func applyAssistantBarItems() {
        guard let state = accessoryState else {
            inputAssistantItem.leadingBarButtonGroups = []
            inputAssistantItem.trailingBarButtonGroups = []
            return
        }

        let leftItems: [UIBarButtonItem] = [
            assistantItem(title: "⌄", identifier: "terminal-key-dismiss", active: false) { [weak self] in
                state.onDismissKeyboard()
                self?.resignFirstResponder()
            },
            assistantItem(title: "⌃", identifier: "terminal-key-ctrl", active: state.ctrlActive) {
                state.onToggleCtrl()
            },
            assistantItem(title: "⌥", identifier: "terminal-key-alt", active: state.altActive) {
                state.onToggleAlt()
            },
            assistantItem(title: "⇥", identifier: "terminal-key-tab", active: false) {
                state.onTab()
            },
            assistantItem(title: "~", identifier: "terminal-key-tilde", active: false) {
                state.onInsertText("~")
            },
            assistantItem(title: "$", identifier: "terminal-key-dollar", active: false) {
                state.onInsertText("$")
            },
            assistantItem(title: "\\", identifier: "terminal-key-backslash", active: false) {
                state.onInsertText("\\")
            }
        ]

        let rightItems: [UIBarButtonItem] = [
            assistantItem(title: "[", identifier: "terminal-key-left-bracket", active: false) {
                state.onInsertText("[")
            },
            assistantItem(title: "]", identifier: "terminal-key-right-bracket", active: false) {
                state.onInsertText("]")
            },
            assistantItem(title: "<", identifier: "terminal-key-less-than", active: false) {
                state.onInsertText("<")
            },
            assistantItem(title: ">", identifier: "terminal-key-greater-than", active: false) {
                state.onInsertText(">")
            },
            assistantItem(title: "←", identifier: "terminal-key-left", active: false) {
                state.onArrowLeft()
            },
            assistantItem(title: "→", identifier: "terminal-key-right", active: false) {
                state.onArrowRight()
            },
            assistantItem(title: "⌘", identifier: "terminal-key-meta", active: state.metaActive) {
                state.onToggleMeta()
            }
        ]

        inputAssistantItem.leadingBarButtonGroups = [
            UIBarButtonItemGroup(barButtonItems: leftItems, representativeItem: nil)
        ]
        inputAssistantItem.trailingBarButtonGroups = [
            UIBarButtonItemGroup(barButtonItems: rightItems, representativeItem: nil)
        ]
    }

    private func assistantItem(
        title: String,
        identifier: String,
        active: Bool,
        action: @escaping () -> Void
    ) -> UIBarButtonItem {
        let item = UIBarButtonItem(title: title, style: .plain, target: nil, action: nil)
        item.primaryAction = UIAction { _ in action() }
        item.accessibilityIdentifier = identifier
        item.tintColor = active ? UIColor.label : UIColor.secondaryLabel
        return item
    }
}

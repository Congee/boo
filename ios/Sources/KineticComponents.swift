import SwiftUI
import UIKit

enum BooTab: String, CaseIterable {
    case terminal
    case history
    case settings

    var icon: String {
        switch self {
        case .terminal: return "terminal"
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
    case scrollLines(Int)
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
    @State private var accumulatedScrollDrag: CGFloat = 0
    @State private var lastDragTranslation: CGSize = .zero

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
                DragGesture(minimumDistance: 4)
                    .onChanged { drag in
                        guard let onGestureAction else { return }
                        let dx = drag.translation.width
                        let dy = drag.translation.height
                        if abs(dy) > abs(dx) {
                            let step = dy - lastDragTranslation.height
                            accumulatedScrollDrag += step
                            let lines = Int(accumulatedScrollDrag / cellHeight)
                            if lines != 0 {
                                onGestureAction(.scrollLines(lines))
                                accumulatedScrollDrag -= CGFloat(lines) * cellHeight
                            }
                        }
                        lastDragTranslation = drag.translation
                    }
                    .onEnded { drag in
                        guard let onGestureAction else { return }
                        let dx = drag.translation.width
                        if abs(dx) >= abs(drag.translation.height) {
                            if dx <= -gestureThreshold {
                                onGestureAction(.arrowRight)
                            } else if dx >= gestureThreshold {
                                onGestureAction(.arrowLeft)
                            }
                        }
                        accumulatedScrollDrag = 0
                        lastDragTranslation = .zero
                    }
            )
        }
        .background(.black)
    }
}

struct RemoteTerminalCanvasView: View {
    let state: DecodedWireScreenState?
    var onGestureAction: ((RemoteTerminalGestureAction) -> Void)? = nil

    private let font = Font.system(size: 14, design: .monospaced)
    private let cellWidth: CGFloat = 8.4
    private let cellHeight: CGFloat = 17
    private let gestureThreshold: CGFloat = 28
    @State private var accumulatedScrollDrag: CGFloat = 0
    @State private var lastDragTranslation: CGSize = .zero

    var body: some View {
        Canvas { context, size in
            guard let state else { return }
            let cols = Int(state.cols)
            let rows = Int(state.rows)
            guard cols > 0, rows > 0, state.cells.count == cols * rows else { return }
            context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.black))
            for row in 0..<rows {
                for col in 0..<cols {
                    let index = row * cols + col
                    let cell = state.cells[index]
                    let x = CGFloat(col) * cellWidth
                    let y = CGFloat(row) * cellHeight
                    if cell.bg_r != 0 || cell.bg_g != 0 || cell.bg_b != 0 || (cell.styleFlags & 0x40) != 0 {
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
                    let color: Color = (cell.styleFlags & 0x20) != 0
                        ? Color(red: Double(cell.fg_r)/255, green: Double(cell.fg_g)/255, blue: Double(cell.fg_b)/255)
                        : .white
                    var text = Text(String(Character(scalar))).font(font).foregroundColor(color)
                    if (cell.styleFlags & 0x01) != 0 { text = text.bold() }
                    if (cell.styleFlags & 0x02) != 0 { text = text.italic() }
                    context.draw(context.resolve(text), at: CGPoint(x: x, y: y), anchor: .topLeading)
                }
            }
            if state.cursorVisible {
                let rect = CGRect(
                    x: CGFloat(state.cursorX) * cellWidth,
                    y: CGFloat(state.cursorY) * cellHeight,
                    width: cellWidth,
                    height: cellHeight
                )
                context.fill(Path(rect), with: .color(.white.opacity(0.45)))
            }
        }
        .background(.black)
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 4)
                .onChanged { drag in
                    guard let onGestureAction else { return }
                    let dx = drag.translation.width
                    let dy = drag.translation.height
                    if abs(dy) > abs(dx) {
                        let step = dy - lastDragTranslation.height
                        accumulatedScrollDrag += step
                        let lines = Int(accumulatedScrollDrag / cellHeight)
                        if lines != 0 {
                            onGestureAction(.scrollLines(lines))
                            accumulatedScrollDrag -= CGFloat(lines) * cellHeight
                        }
                    }
                    lastDragTranslation = drag.translation
                }
                .onEnded { drag in
                    guard let onGestureAction else { return }
                    let dx = drag.translation.width
                    if abs(dx) >= abs(drag.translation.height) {
                        if dx <= -gestureThreshold {
                            onGestureAction(.arrowRight)
                        } else if dx >= gestureThreshold {
                            onGestureAction(.arrowLeft)
                        }
                    }
                    accumulatedScrollDrag = 0
                    lastDragTranslation = .zero
                }
        )
    }
}

struct TerminalKeyboardBridge: UIViewRepresentable {
    @Binding var isFocused: Bool
    let onText: (String) -> Void
    let onBackspace: () -> Void
    let onKeyCommand: (String, UIKeyModifierFlags) -> Bool
    let accessoryState: TerminalKeyboardAccessoryState

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> TerminalProxyTextView {
        let textView = TerminalProxyTextView(frame: .zero, textContainer: nil)
        textView.delegate = context.coordinator
        textView.onText = onText
        textView.onBackspace = onBackspace
        textView.onKeyCommand = onKeyCommand
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
        uiView.onKeyCommand = onKeyCommand
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
    let onInsertText: (String) -> Void
    let onEscape: () -> Void
    let onCtrlModifierEvent: (TerminalAssistantModifierEvent) -> Void
    let onAltModifierEvent: (TerminalAssistantModifierEvent) -> Void
    let onMetaModifierEvent: (TerminalAssistantModifierEvent) -> Void
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

enum TerminalAssistantModifierEvent {
    case pressBegan
    case pressEnded(wasTap: Bool)
}

final class TerminalProxyTextView: UITextView {
    var onText: ((String) -> Void)?
    var onBackspace: (() -> Void)?
    var onKeyCommand: ((String, UIKeyModifierFlags) -> Bool)?
    private var desiredFocus = false
    private var accessoryState: TerminalKeyboardAccessoryState?
    private var assistantControls: [String: TerminalAssistantKeyControl] = [:]

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
        configureAssistantBar()
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
        _ = becomeFirstResponder()
        return true
    }

    override var keyCommands: [UIKeyCommand]? {
        let letters = (0..<26).compactMap { UnicodeScalar(65 + $0).map { String(Character($0)) } }
        let controlLetters = letters.map { input in
            UIKeyCommand(input: input.lowercased(), modifierFlags: .control, action: #selector(handleKeyCommand(_:)))
        }
        let altLetters = letters.map { input in
            UIKeyCommand(input: input.lowercased(), modifierFlags: .alternate, action: #selector(handleKeyCommand(_:)))
        }
        let commandLetters = letters.map { input in
            UIKeyCommand(input: input.lowercased(), modifierFlags: .command, action: #selector(handleKeyCommand(_:)))
        }
        let specialInputs = [
            (UIKeyCommand.inputUpArrow, UIKeyModifierFlags([])),
            (UIKeyCommand.inputDownArrow, UIKeyModifierFlags([])),
            (UIKeyCommand.inputLeftArrow, UIKeyModifierFlags([])),
            (UIKeyCommand.inputRightArrow, UIKeyModifierFlags([])),
            ("\t", UIKeyModifierFlags([])),
            ("\u{1b}", UIKeyModifierFlags([])),
            ("\r", UIKeyModifierFlags([]))
        ].map { input, modifiers in
            UIKeyCommand(input: input, modifierFlags: modifiers, action: #selector(handleKeyCommand(_:)))
        }
        return controlLetters + altLetters + commandLetters + specialInputs
    }

    @objc private func handleKeyCommand(_ sender: UIKeyCommand) {
        guard let input = sender.input else { return }
        _ = onKeyCommand?(input, sender.modifierFlags)
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
    
    private func configureAssistantBar() {
        let leftItems = [
            assistantItem(title: "⌃", identifier: "terminal-key-ctrl", repeatable: false, role: .modifier),
            assistantItem(title: "⌥", identifier: "terminal-key-alt", repeatable: false, role: .modifier),
            assistantItem(title: "⇥", identifier: "terminal-key-tab", repeatable: true, role: .regular),
            assistantItem(title: "~", identifier: "terminal-key-tilde", repeatable: true, role: .regular),
            assistantItem(title: "$", identifier: "terminal-key-dollar", repeatable: true, role: .regular),
            assistantItem(title: "\\", identifier: "terminal-key-backslash", repeatable: true, role: .regular)
        ]
        let rightItems = [
            assistantItem(title: "[", identifier: "terminal-key-left-bracket", repeatable: true, role: .regular),
            assistantItem(title: "]", identifier: "terminal-key-right-bracket", repeatable: true, role: .regular),
            assistantItem(title: "<", identifier: "terminal-key-less-than", repeatable: true, role: .regular),
            assistantItem(title: ">", identifier: "terminal-key-greater-than", repeatable: true, role: .regular),
            assistantItem(title: "←", identifier: "terminal-key-left", repeatable: true, role: .regular),
            assistantItem(title: "→", identifier: "terminal-key-right", repeatable: true, role: .regular),
            assistantItem(title: "⌘", identifier: "terminal-key-meta", repeatable: false, role: .modifier)
        ]

        inputAssistantItem.leadingBarButtonGroups = [
            UIBarButtonItemGroup(barButtonItems: leftItems, representativeItem: nil)
        ]
        inputAssistantItem.trailingBarButtonGroups = [
            UIBarButtonItemGroup(barButtonItems: rightItems, representativeItem: nil)
        ]
    }

    private func applyAssistantBarItems() {
        guard let state = accessoryState else {
            assistantControls.values.forEach { $0.update(action: nil, modifierHandler: nil, isActive: false) }
            return
        }

        assistantControls["terminal-key-ctrl"]?.update(
            action: nil,
            modifierHandler: { state.onCtrlModifierEvent($0) },
            isActive: state.ctrlActive
        )
        assistantControls["terminal-key-alt"]?.update(
            action: nil,
            modifierHandler: { state.onAltModifierEvent($0) },
            isActive: state.altActive
        )
        assistantControls["terminal-key-tab"]?.update(action: state.onTab, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-tilde"]?.update(action: { state.onInsertText("~") }, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-dollar"]?.update(action: { state.onInsertText("$") }, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-backslash"]?.update(action: { state.onInsertText("\\") }, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-left-bracket"]?.update(action: { state.onInsertText("[") }, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-right-bracket"]?.update(action: { state.onInsertText("]") }, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-less-than"]?.update(action: { state.onInsertText("<") }, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-greater-than"]?.update(action: { state.onInsertText(">") }, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-left"]?.update(action: state.onArrowLeft, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-right"]?.update(action: state.onArrowRight, modifierHandler: nil, isActive: false)
        assistantControls["terminal-key-meta"]?.update(
            action: nil,
            modifierHandler: { state.onMetaModifierEvent($0) },
            isActive: state.metaActive
        )
    }

    private enum AssistantKeyRole {
        case regular
        case modifier
    }

    private func assistantItem(
        title: String,
        identifier: String,
        repeatable: Bool,
        role: AssistantKeyRole
    ) -> UIBarButtonItem {
        let control = TerminalAssistantKeyControl(
            title: title,
            identifier: identifier,
            repeatable: repeatable,
            role: role == .modifier ? .modifier : .regular
        )
        assistantControls[identifier] = control
        return UIBarButtonItem(customView: control)
    }
}

final class TerminalAssistantKeyControl: UIControl {
    enum Role {
        case regular
        case modifier
    }

    private let label = UILabel()
    private let repeatable: Bool
    private let role: Role
    private var action: (() -> Void)?
    private var modifierHandler: ((TerminalAssistantModifierEvent) -> Void)?
    private var repeatTimer: Timer?
    private var touchBeganAt: Date?
    private var isActive = false

    private static let initialRepeatDelay: TimeInterval = 0.42
    private static let repeatInterval: TimeInterval = 0.08
    private static let tapThreshold: TimeInterval = 0.20

    init(title: String, identifier: String, repeatable: Bool, role: Role) {
        self.repeatable = repeatable
        self.role = role
        super.init(frame: CGRect(x: 0, y: 0, width: 30, height: 32))
        accessibilityIdentifier = identifier
        isExclusiveTouch = false
        label.text = title
        label.font = UIFont.systemFont(ofSize: 17, weight: .semibold)
        label.textAlignment = .center
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor),
            label.trailingAnchor.constraint(equalTo: trailingAnchor),
            label.topAnchor.constraint(equalTo: topAnchor),
            label.bottomAnchor.constraint(equalTo: bottomAnchor),
            widthAnchor.constraint(greaterThanOrEqualToConstant: 28),
            heightAnchor.constraint(equalToConstant: 32)
        ])
        updateColors()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func update(
        action: (() -> Void)?,
        modifierHandler: ((TerminalAssistantModifierEvent) -> Void)?,
        isActive: Bool
    ) {
        self.action = action
        self.modifierHandler = modifierHandler
        self.isActive = isActive
        updateColors()
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesBegan(touches, with: event)
        touchBeganAt = Date()
        switch role {
        case .modifier:
            modifierHandler?(.pressBegan)
        case .regular:
            action?()
            startRepeatIfNeeded()
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        endInteraction(cancelled: false)
        super.touchesEnded(touches, with: event)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        endInteraction(cancelled: true)
        super.touchesCancelled(touches, with: event)
    }

    private func endInteraction(cancelled: Bool) {
        stopRepeat()
        let duration = Date().timeIntervalSince(touchBeganAt ?? Date())
        touchBeganAt = nil
        if role == .modifier {
            modifierHandler?(.pressEnded(wasTap: !cancelled && duration <= Self.tapThreshold))
        }
    }

    private func startRepeatIfNeeded() {
        guard repeatable else { return }
        repeatTimer = Timer.scheduledTimer(withTimeInterval: Self.initialRepeatDelay, repeats: false) { [weak self] _ in
            guard let self else { return }
            self.repeatTimer = Timer.scheduledTimer(withTimeInterval: Self.repeatInterval, repeats: true) { [weak self] _ in
                self?.action?()
            }
            RunLoop.main.add(self.repeatTimer!, forMode: .common)
        }
        if let repeatTimer {
            RunLoop.main.add(repeatTimer, forMode: .common)
        }
    }

    private func stopRepeat() {
        repeatTimer?.invalidate()
        repeatTimer = nil
    }

    private func updateColors() {
        label.textColor = isActive ? .label : .secondaryLabel
    }
}

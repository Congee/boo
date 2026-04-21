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

    var body: some View {
        VStack(alignment: .leading, spacing: KineticSpacing.xs) {
            HStack {
                Text("boo")
                    .font(.system(size: 20, weight: .black, design: .monospaced))
                    .foregroundStyle(KineticColor.primary)
                Spacer()
            }
            Text(title)
                .font(KineticFont.headline)
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
        .padding(.top, KineticSpacing.lg)
        .padding(.bottom, KineticSpacing.md)
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
                SecureField(placeholder, text: $text)
                    .accessibilityIdentifier(accessibilityIdentifier ?? placeholder)
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
    var onTap: (() -> Void)? = nil
    var accessibilityIdentifier: String? = nil

    var body: some View {
        Button {
            onTap?()
        } label: {
            HStack(spacing: KineticSpacing.md) {
                Image(systemName: icon)
                    .font(.system(size: 20))
                    .foregroundStyle(KineticColor.primary)
                    .frame(width: 40, height: 40)
                    .background(KineticColor.surfaceContainerHighest)
                    .clipShape(RoundedRectangle(cornerRadius: KineticRadius.button))
                VStack(alignment: .leading, spacing: KineticSpacing.xs) {
                    Text(title)
                        .font(KineticFont.bodySmall)
                        .fontWeight(.bold)
                        .foregroundStyle(KineticColor.onSurface)
                    Text(subtitle)
                        .font(KineticFont.caption)
                        .foregroundStyle(KineticColor.onSurfaceVariant)
                }
                Spacer()
                Image(systemName: "chevron.right")
                    .foregroundStyle(KineticColor.onSurfaceVariant)
            }
            .padding(KineticSpacing.md)
            .containerCard()
        }
        .buttonStyle(.plain)
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
                        } else {
                            if dx <= -gestureThreshold {
                                onGestureAction(.arrowLeft)
                            } else if dx >= gestureThreshold {
                                onGestureAction(.arrowRight)
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

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> UITextField {
        let textField = UITextField(frame: .zero)
        textField.delegate = context.coordinator
        textField.autocorrectionType = .no
        textField.autocapitalizationType = .none
        textField.spellCheckingType = .no
        textField.smartQuotesType = .no
        textField.smartDashesType = .no
        textField.smartInsertDeleteType = .no
        textField.returnKeyType = .default
        textField.tintColor = .clear
        textField.textColor = .clear
        textField.backgroundColor = .clear
        return textField
    }

    func updateUIView(_ uiView: UITextField, context: Context) {
        if isFocused, !uiView.isFirstResponder {
            uiView.becomeFirstResponder()
        } else if !isFocused, uiView.isFirstResponder {
            uiView.resignFirstResponder()
        }
    }

    final class Coordinator: NSObject, UITextFieldDelegate {
        let parent: TerminalKeyboardBridge

        init(parent: TerminalKeyboardBridge) {
            self.parent = parent
        }

        func textFieldShouldReturn(_ textField: UITextField) -> Bool {
            parent.onText("\r")
            return false
        }

        func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
            if range.length > 0 && string.isEmpty {
                for _ in 0..<range.length {
                    parent.onBackspace()
                }
                return false
            }

            guard !string.isEmpty else { return false }
            parent.onText(string)
            return false
        }
    }
}

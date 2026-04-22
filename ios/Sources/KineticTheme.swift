import SwiftUI

enum KineticColor {
    static let surface = Color(red: 14/255, green: 14/255, blue: 14/255)
    static let surfaceContainer = Color(red: 26/255, green: 25/255, blue: 25/255)
    static let surfaceContainerHigh = Color(red: 32/255, green: 31/255, blue: 31/255)
    static let surfaceContainerHighest = Color(red: 38/255, green: 38/255, blue: 38/255)
    static let surfaceContainerLowest = Color.black
    static let primary = Color(red: 105/255, green: 218/255, blue: 255/255)
    static let primaryContainer = Color(red: 0/255, green: 207/255, blue: 252/255)
    static let secondary = Color(red: 229/255, green: 226/255, blue: 225/255)
    static let onSurface = Color.white
    static let onSurfaceVariant = Color(red: 173/255, green: 170/255, blue: 170/255)
    static let error = Color(red: 255/255, green: 113/255, blue: 108/255)
    static let success = Color(red: 110/255, green: 231/255, blue: 183/255)
    static let tertiary = Color(red: 118/255, green: 150/255, blue: 253/255)
}

enum KineticFont {
    static let heroTitle = Font.system(size: 40, weight: .black)
    static let headline = Font.system(size: 24, weight: .bold)
    static let sectionLabel = Font.system(size: 11, weight: .bold)
    static let body = Font.system(size: 16, weight: .medium)
    static let bodySmall = Font.system(size: 14, weight: .medium)
    static let caption = Font.system(size: 12, weight: .medium)
    static let monoData = Font.system(size: 14, weight: .regular, design: .monospaced)
    static let monoInput = Font.system(size: 16, weight: .medium, design: .monospaced)
}

enum KineticSpacing {
    static let xs: CGFloat = 4
    static let sm: CGFloat = 8
    static let md: CGFloat = 16
    static let lg: CGFloat = 24
    static let xl: CGFloat = 36
    static let xxl: CGFloat = 48
}

enum KineticRadius {
    static let button: CGFloat = 8
    static let container: CGFloat = 16
    static let large: CGFloat = 24
}

extension View {
    func containerCard(color: Color = KineticColor.surfaceContainer) -> some View {
        background(color)
            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.container))
    }
}

struct KineticPrimaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(KineticFont.body)
            .fontWeight(.black)
            .foregroundStyle(KineticColor.surface)
            .frame(maxWidth: .infinity)
            .padding(.vertical, KineticSpacing.md)
            .background(
                LinearGradient(
                    colors: [KineticColor.primary, KineticColor.primaryContainer],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            )
            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.large))
            .scaleEffect(configuration.isPressed ? 0.98 : 1.0)
            .animation(.easeOut(duration: 0.15), value: configuration.isPressed)
    }
}

struct KineticSecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(KineticFont.body)
            .fontWeight(.bold)
            .foregroundStyle(KineticColor.secondary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, KineticSpacing.md)
            .background(KineticColor.surfaceContainerHighest)
            .clipShape(RoundedRectangle(cornerRadius: KineticRadius.large))
            .scaleEffect(configuration.isPressed ? 0.98 : 1.0)
            .animation(.easeOut(duration: 0.15), value: configuration.isPressed)
    }
}

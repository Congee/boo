import Foundation

@main
struct RemoteValidatorMain {
    static func main() {
        let args = resolveRemoteValidatorArgs()
        let validator = RemoteValidator()

        do {
            try validator.connect(host: args.host, port: args.port)
            try validator.validateRoundTrip()
            validator.disconnect()
            print("iOS remote daemon validation passed")
        } catch {
            validator.disconnect()
            FileHandle.standardError.write(Data("validation failed: \(error)\n".utf8))
            exit(1)
        }
    }
}

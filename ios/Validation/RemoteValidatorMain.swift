import Foundation

@main
struct RemoteValidatorMain {
    static func main() {
        let args = resolveRemoteValidatorArgs()
        let validator = RemoteValidator(authKey: args.authKey)

        do {
            if args.checkDiscovery {
                if let endpoint = validator.browse() {
                    FileHandle.standardError.write(Data("discovered Bonjour endpoint: \(endpoint)\n".utf8))
                } else {
                    FileHandle.standardError.write(Data("warning: Bonjour discovery did not resolve within timeout\n".utf8))
                }
            }
            if args.expectAuthFailure {
                try validator.validateAuthenticationFailure(host: args.host, port: args.port)
            } else {
                try validator.connect(host: args.host, port: args.port)
                try validator.validateRoundTrip()
            }
            validator.disconnect()
            print("iOS remote daemon validation passed")
        } catch {
            validator.disconnect()
            FileHandle.standardError.write(Data("validation failed: \(error)\n".utf8))
            exit(1)
        }
    }
}

# Eclipse JDT Language Server

Java completion, diagnostics and navigation powered by Eclipse JDT LS.

Open a supported file in a trusted workspace to start the Loog WASM client.

JDT LS 1.63.0 requires a JDK 25 or newer to run. Set `JAVA_HOME` or put its
`java` on PATH. Projects can target older Java versions.
The launcher uses `loog.runtime.node` and starts JDT LS with a private,
writable workspace for each session. Open the Maven or Gradle project root
for project-aware features.

See the [upstream server documentation](https://github.com/eclipse-jdtls/eclipse.jdt.ls).

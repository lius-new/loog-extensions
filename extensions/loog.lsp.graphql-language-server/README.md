# GraphQL Language Server

Schema-aware diagnostics, completion and navigation for GraphQL.

Open a supported file in a trusted workspace to start the Loog WASM client.
The server runs through `loog.runtime.node`.

Configure the schema and operations in a GraphQL Config file, such as
`.graphqlrc.json`. For example, `{"schema":"schema.graphql","documents":"operations/**/*.graphql"}`
uses a local schema and the operation files under `operations/`.

See the [upstream server documentation](https://github.com/graphql/graphiql/tree/main/packages/graphql-language-service-cli).

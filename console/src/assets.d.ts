// Files the build copies into assets/ under a name made from their contents;
// importing one gives its URL.
declare module "*.wasm" {
  const url: string;
  export default url;
}

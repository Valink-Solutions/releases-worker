# ChunkVault Release Worker

This is a Release worker for ChunkVault Desktop. It is designed to look at the GitHub releases and return it as an endpoint for Tauri to use.

The worker is not a standalone tool, but a part of the ChunkVault project. It is currently public for contributions only.

The main functionality of this worker is to fetch the latest release data from a specific GitHub repository and provide it in a format that can be easily consumed by Tauri applications.

In the future, we may consider expanding its functionality to support other use cases.

## Development

### Wrangler

Wrangler is used to develop, deploy, and configure your Worker via CLI.

Further documentation for Wrangler can be found [here](https://developers.cloudflare.com/workers/tooling/wrangler).

want to setup with a similar template to this one? [Click here](https://github.com/cloudflare/workers-sdk/tree/main/templates/experimental/worker-rust).

### Usage

This template starts you off with a `src/lib.rs` file, acting as an entrypoint for requests hitting your Worker. Feel free to add more code in this file, or create Rust modules anywhere else for this project to use.

With `wrangler`, you can build, test, and deploy your Worker with the following commands:

```sh
# run your Worker in an ideal development workflow (with a local server, file watcher & more)
$ npm run dev

# deploy your Worker globally to the Cloudflare network (update your wrangler.toml file for configuration)
$ npm run deploy
```

Read the latest `worker` crate documentation here: https://docs.rs/worker

### WebAssembly

`workers-rs` (the Rust SDK for Cloudflare Workers used in this template) is meant to be executed as compiled WebAssembly, and as such so **must** all the code you write and depend upon. All crates and modules used in Rust-based Workers projects have to compile to the `wasm32-unknown-unknown` triple.

Read more about this on the [`workers-rs`](https://github.com/cloudflare/workers-rs) project README.



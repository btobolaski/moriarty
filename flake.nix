{
  description = "moriarty - Claude Code and pi log analysis CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    flake-utils,
    advisory-db,
    rust-overlay,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };

        inherit (pkgs) lib;

        # On Linux, build against musl so the resulting binaries are fully
        # statically linked. The dependency tree is pure Rust, so rustc's
        # self-contained musl runtime suffices; no C cross toolchain needed.
        # nixpkgs' rustc only ships the native target's std, hence rust-overlay.
        muslTargets = {
          "x86_64-linux" = "x86_64-unknown-linux-musl";
          "aarch64-linux" = "aarch64-unknown-linux-musl";
        };
        muslTarget = muslTargets.${system} or null;

        craneLib =
          if muslTarget != null
          then
            (crane.mkLib pkgs).overrideToolchain (p:
              p.rust-bin.stable.latest.default.override {
                targets = [muslTarget];
              })
          else crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;

        # Common arguments can be set here to avoid repeating them later
        commonArgs =
          {
            inherit src;
            strictDeps = true;
          }
          # if/else rather than two optionalAttrs merges so the branches can
          # never both set CARGO_BUILD_RUSTFLAGS: a later `//` merge would
          # silently clobber the musl flags.
          // (
            if muslTarget != null
            then {
              CARGO_BUILD_TARGET = muslTarget;
              # musl targets default to +crt-static; set it explicitly so the
              # static-linking intent survives toolchain changes.
              CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
            }
            else
              lib.optionalAttrs pkgs.stdenv.isDarwin {
                # Fully static binaries are impossible on macOS (libSystem must
                # be dynamic), so the goal here is portability: no nix-store
                # dylib references. rustc propagates a store libiconv onto
                # every link line even though no symbol binds to it; dead-
                # stripping unused dylibs removes that reference so the binary
                # runs on any Mac.
                CARGO_BUILD_RUSTFLAGS = "-C link-arg=-Wl,-dead_strip_dylibs";
              }
          );

        # Build *just* the cargo dependencies (of the entire workspace),
        # so we can reuse all of that work (e.g. via cachix) when running in CI
        # It is *highly* recommended to use something like cargo-hakari to avoid
        # cache misses when building individual top-level-crates
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        individualCrateArgs =
          commonArgs
          // {
            inherit cargoArtifacts;
            inherit (craneLib.crateNameFromCargoToml {inherit src;}) version;
            # NB: we disable tests since we'll run them all via cargo-nextest
            doCheck = false;
          };

        fileSetForCrate = crate:
          lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              (craneLib.fileset.commonCargoSources ./crates/my-workspace-hack)
              (craneLib.fileset.commonCargoSources crate)
            ];
          };

        # Build the top-level crates of the workspace as individual derivations.
        # This allows consumers to only depend on (and build) only what they need.
        # Though it is possible to build the entire workspace as a single derivation,
        # so this is left up to you on how to organize things
        #
        # Note that the cargo workspace must define `workspace.members` using wildcards,
        # otherwise, omitting a crate (like we do below) will result in errors since
        # cargo won't be able to find the sources for all members.
        moriarty = craneLib.buildPackage (
          individualCrateArgs
          // {
            pname = "moriarty";
            cargoExtraArgs = "-p moriarty";
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                (craneLib.fileset.commonCargoSources ./crates/my-workspace-hack)
                (craneLib.fileset.commonCargoSources ./crates/claude_logs)
                (craneLib.fileset.commonCargoSources ./crates/pi_logs)
                (craneLib.fileset.commonCargoSources ./crates/cost_analyzer)
                (craneLib.fileset.commonCargoSources ./crates/moriarty)
                ./doc/man
              ];
            };
            nativeBuildInputs = [pkgs.installShellFiles];
            postInstall = ''
              installManPage doc/man/moriarty.1
              installManPage doc/man/moriarty-tool-rules.5
              installManPage doc/man/moriarty-bash-rules.5
            '';
          }
        );
      in {
        checks = {
          # Build the crates as part of `nix flake check` for convenience
          inherit moriarty;

          # Run clippy (and deny all warnings) on the workspace source,
          # again, reusing the dependency artifacts from above.
          #
          # Note that this is done as a separate derivation so that
          # we can block the CI if there are issues here, but not
          # prevent downstream consumers from building our crate by itself.
          my-workspace-clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          my-workspace-doc = craneLib.cargoDoc (
            commonArgs
            // {
              inherit cargoArtifacts;
              # This can be commented out or tweaked as necessary, e.g. set to
              # `--deny rustdoc::broken-intra-doc-links` to only enforce that lint
              env.RUSTDOCFLAGS = "--deny warnings";
            }
          );

          # Check formatting
          my-workspace-fmt = craneLib.cargoFmt {
            inherit src;
          };

          # Audit dependencies
          my-workspace-audit = craneLib.cargoAudit {
            inherit src advisory-db;
          };

          # Audit licenses
          my-workspace-deny = craneLib.cargoDeny {
            inherit src;
          };

          # Run tests with cargo-nextest under the `nix` profile, which is
          # defined in .config/nextest.toml and skips tests that depend on
          # sandbox-incompatible host state (system zoneinfo, $HOME-based
          # XDG fallbacks, etc.). Those tests still run under `cargo nextest
          # run` locally where the host environment is available.
          my-workspace-nextest = craneLib.cargoNextest (
            commonArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass --profile nix";
            }
          );

          # Ensure that cargo-hakari is up to date
          my-workspace-hakari = craneLib.mkCargoDerivation {
            inherit src;
            pname = "my-workspace-hakari";
            cargoArtifacts = null;
            doInstallCargoArtifacts = false;

            buildPhaseCargoCommand = ''
              cargo hakari generate --diff  # workspace-hack Cargo.toml is up-to-date
              cargo hakari manage-deps --dry-run  # all workspace crates depend on workspace-hack
              cargo hakari verify
            '';

            nativeBuildInputs = [
              pkgs.cargo-hakari
            ];
          };
        };

        packages = {
          inherit moriarty;
        };

        apps = {
          moriarty = flake-utils.lib.mkApp {
            drv = moriarty;
          };
        };

        devShells.default = craneLib.devShell {
          # Inherit inputs from checks.
          checks = self.checks.${system};

          # Extra inputs can be added here; cargo and rustc are provided by default.
          packages = with pkgs; [
            alejandra
            cargo-hakari
            prettier
            treefmt
          ];
        };
      }
    );
}

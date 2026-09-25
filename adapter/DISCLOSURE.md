# Veto code and prior-work disclosure

Last updated: 2026-09-25.

## Licence

Veto is licensed under the GNU Affero General Public License v3.0 (see
`LICENSE`). Its client tools and console link the Swig client crates, which are
AGPL-3.0; the hosted console offers this repository as its source.

## Third-party code

- **Swig** ([`anagrambuild/swig-wallet`](https://github.com/anagrambuild/swig-wallet),
  AGPL-3.0, copyright Anagram). Veto uses Swig's `swig-interface` and
  `swig-state` crates as Cargo dependencies pinned to commit
  `3e0411f0c2980b26296903eaa4aa0c2d07869316`, and runs against the official
  Swig program deployed at `swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB`. No Swig
  source is copied into this repository. The on-chain Veto gate does not link
  Swig code.
- Rust and npm dependencies are pinned by the checked-in lockfiles.

## History of this code

Veto was developed during the Crypto World's Fair contest period (from
2026-09-23) inside a local checkout of the Swig repository, then moved into this
standalone repository on 2026-09-25. Evidence logs in `verification-logs/`
from before the move reference that checkout's paths.

During that period a local change to Swig's compact-instruction parser (moving
per-instruction account metadata from stack arrays into owned vectors, keeping
`MAX_ACCOUNTS=254`) was made so a locally built Swig program would fit the SBF
stack frame. That change was never upstreamed, is not part of this repository,
and is not needed: the flows pass unchanged against the official Swig program
(`verification-logs/2026-09-24-official-swig-clone-*.log` and later runs).

The relay fixture in `adapter/fixtures/relay` was written for this repository;
an earlier version of the same test relayed through Swig's Apache-2.0
`test-program-authority` crate.

## Evidence boundary

Execution so far used disposable identities, local validator funds, devnet test
SOL, and valueless test tokens. There has been no mainnet deployment,
independent audit, maintainer acceptance, customer validation, or proof of
business demand. Veto constrains the Swig route and the programs reachable from
it; it does not judge what approved code does.

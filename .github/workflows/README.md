# GitHub Actions CI Pipeline

## Architecture

```
pr.yml (pull_request -> main)
├── lint          [REQUIRED]  inline fmt+clippy
├── security      [REQUIRED]  inline cargo audit + npm audit
├── build         [REQUIRED]  inline cargo check + cargo test (hardened)
├── tf-lint       [REQUIRED]  inline terraform fmt+validate
├── rust-deep     [info]      -> ci-rust.yml (fmt, clippy, test, wasm-build)
├── security-deep [info]      -> ci-security.yml (audit, gitleaks, trivy, npm-audit)
├── tf-modules    [info]      -> ci-terraform.yml (fmt+validate matrix [root, aws])
└── integration   [info]      runs integration tests via wrangler dev

security-scan.yml (schedule Mon 06:00 + manual)
├── scan                     -> ci-security.yml
└── audit-json               cargo audit --json (artifact)

deploy.yml (push -> main + manual)
├── lint                     -> ci-rust.yml
├── security                 -> ci-security.yml
├── build (inline)           cargo check --target wasm32
├── deploy-auth              needs: build
├── deploy-gateway           needs: [build, deploy-auth]
├── deploy-analytics         needs: [build, deploy-gateway]
└── smoke                    needs: [deploy-auth, deploy-gateway, deploy-analytics]
```

## Required vs Informational Checks

Branch protection on `main` requires exact job name match:

| Job ID (pr.yml) | Status | Notes |
|-----------------|--------|-------|
| `lint` | REQUIRED | inline — must match exactly |
| `security` | REQUIRED | inline — must match exactly |
| `build` | REQUIRED | inline — must match exactly |
| `tf-lint` | REQUIRED | inline — must match exactly |
| `rust-deep` | informational | calls ci-rust.yml |
| `security-deep` | informational | calls ci-security.yml |
| `tf-modules` | informational | calls ci-terraform.yml |
| `integration` | REQUIRED | inline — boots wrangler dev, runs gateway+auth+analytics suites |

⚠️ **Branch-protection coupling**: When a job calls a reusable workflow (`uses:`),
the check name becomes `parent-job / child-job` (e.g. `rust-deep / fmt`).
This breaks exact-name required checks. The 5 required jobs stay inline to
preserve the branch-protection names.

## How to Add a New Reusable Template

1. Create `.github/workflows/ci-<name>.yml` with `on: workflow_call`
2. Add named job(s) following existing conventions
3. Call it from pr.yml and/or deploy.yml with `uses: ./.github/workflows/ci-<name>.yml`
4. If the new check should be REQUIRED for branch protection:
   - Add a top-level job in pr.yml (not a `uses:` call)
   - Duplicate the critical commands inline
   - Update branch protection rules in GitHub repo settings

## Design Decisions

- **deploy.yml build kept inline**: ci-rust.yml runs `cargo build --release --target wasm32`,
  which is redundant with each deploy worker's `worker-build --release`. Inline build
  runs `cargo check` only — faster, same gate value.
- **SHA pinning**: ci-security.yml pins all third-party actions to commit SHA with
  version comments. Other workflows use version tags (lower maintenance burden for
  non-security paths).
- **SARIF upload**: Uses `continue-on-error: true` because the repo may lack
  GitHub Advanced Security. The `exit-code: 1` on trivy table scan enforces the
  actual pass/fail.
- **No secrets passed to templates**: ci-rust/ci-security/ci-terraform use only
  auto-available `GITHUB_TOKEN`. Deploy-specific secrets (CLOUDFLARE_*) stay in
  deploy.yml.

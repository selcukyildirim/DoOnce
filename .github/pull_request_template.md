## Summary

<!-- What changed and why? -->

## Threat and compliance impact

- [ ] No new trust boundary, secret, PII/PHI/cardholder data or outbound network path
- [ ] New trust boundary / network path documented in `docs/security/threat-model.md`
- [ ] Authorization and tenant isolation reviewed
- [ ] Audit events added or updated
- [ ] Secret redaction verified
- [ ] Migration/rollback plan included (if schema changed)
- [ ] No AGPL code introduced (see ADR 0002)

## Testing

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo deny check`
- [ ] Security regression test added (for security fixes)

## Notes for reviewers

<!-- Anything risky, intentionally deferred, or needing a second opinion? -->

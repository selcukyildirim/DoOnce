# ADR 0009 — OIDC SSO, sessions, MFA and SCIM

- Status: Accepted
- Date: 2026-11-06

## Context

Phase 4 delivered API-key auth and RBAC. Enterprise customers also require human
SSO, browser sessions with CSRF protection, MFA and automated user provisioning.
SAML and WebAuthn are substantial separate integrations; we need a secure,
shippable identity increment without weakening the API-key path.

## Decision

1. **OIDC Authorization Code + PKCE** is the human SSO path. Discovery, ID-token
   signature validation, issuer/audience checks and nonce validation are handled
   by the `openidconnect` crate. The client secret is encrypted at rest.
2. **Just-in-time provisioning** creates missing users with the `viewer` role.
   Linking an IdP identity to an existing email requires a verified email claim.
   Admins can elevate roles afterwards.
3. **Browser sessions** use an opaque 256-bit token in an HttpOnly,
   SameSite=Strict cookie. Only a SHA-256 hash is stored server-side. Sessions
   expire (`DOONCE_SESSION_TTL_SECS`, default 8h) and are revocable.
4. **CSRF protection** uses a non-HttpOnly `doonce_csrf` cookie and an
   `x-csrf-token` header. Session-authenticated mutations must match; API-key
   requests are exempt because they are not ambient cookies.
5. **MFA gate.** When a user has a confirmed TOTP factor, new sessions are
   marked unverified and can only access `/auth/me`, `/auth/logout` and
   `/auth/mfa/*`. A valid TOTP code upgrades the session.
6. **TOTP** secrets are generated with a CSPRNG, encrypted with the tenant
   master key and never returned after enrollment.
7. **SCIM 2.0** user list/filter/create/read/patch/deprovision is exposed at
   `/scim/v2/Users`. SCIM requests require an API key with the `scim` scope (or
   an admin browser session). Deprovisioning sets the user status to `deleted`.
8. **Secure cookies in production.** `DOONCE_PUBLIC_URL` drives the OIDC
   redirect URI and `Secure` cookie behavior. HTTPS is mandatory outside local
   development.
9. **Deferred:** SAML 2.0 and WebAuthn/passkeys are the next identity
   sub-phase. They must reuse the same session, CSRF and audit model.

## Security notes

- Redirect URIs and issuers are configured per tenant; discovery is HTTPS-only.
- The HTTP client used for discovery/token exchange does not follow redirects.
- Identity-provider secrets are encrypted and never returned by the API.
- Audit events cover login, logout, MFA changes, IdP changes and SCIM
  provisioning.
- **Known advisory:** `openidconnect` pulls in `rsa 0.9.10`
  (RUSTSEC-2023-0071). The vulnerable RSA decryption path is not exposed; the
  crate is used for public-key ID-token signature verification. We pin trusted
  issuers, require HTTPS, use short token lifetimes and track an upstream
  replacement. The exception is recorded in `deny.toml`.

## Consequences

- Humans can use SSO; service accounts continue to use API keys.
- Session/CSRF/MFA enforcement is centralized in the auth extractor, so new
  routes inherit it.
- SCIM tokens must be created deliberately with the `scim` scope.
- SAML/WebAuthn remain explicit gaps rather than hidden assumptions.

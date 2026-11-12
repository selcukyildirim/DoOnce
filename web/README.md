# DoOnce Web Console

React + Vite control-plane UI for the DoOnce API.

```bash
cd web
npm install
npm run build     # outputs web/dist, served by `doonce serve`
npm run dev       # Vite dev server on :5173 (proxy API in production via same origin)
```

The console stores the API key in `localStorage` and sends it as `x-api-key`.
It never renders credential secrets. For SSO/SCIM/MFA, see the auth roadmap
(Phase 4+).

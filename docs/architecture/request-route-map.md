# Request & Route Map

Every HTTP request passes through one middleware layer before reaching a handler.

```mermaid
flowchart LR
    req([HTTP Request]) --> rts["redirect_trailing_slash\n(outermost .layer)"]
    rts -->|"path ends with /"| red(["308 Permanent Redirect"])
    rts --> router{Router}

    router --> pub[Public Routes]
    router --> adm["/admin/**\nbasic_auth (.route_layer)"]

    pub --> r1["GET /  →  redirect /guide"]
    pub --> r2["GET /health"]
    pub --> r3["GET /guide"]
    pub --> r4["GET /guide/partial  (HTMX)"]
    pub --> r5["GET /channel/:id/tune"]
    pub --> r6["GET /channel/:id/next"]
    pub --> r7["GET /stream-proxy"]

    adm --> a1["GET+POST /admin/channels\nGET /admin/channels/new\nGET+POST /admin/channels/:id\nGET /admin/channels/:id/edit\nPOST /admin/channels/:id/delete"]
    adm --> a2["POST /admin/channels/:id/sources\nPOST /admin/sources/:id/delete\nPOST /admin/sources/:id/toggle\nPOST /admin/sources/:id/test"]
    adm --> a3["POST /admin/channels/:id/playlist\nPOST /admin/playlist/:id/delete"]
    adm --> a4["GET /admin/discover\nPOST /admin/discover/add-form\nPOST /admin/discover/add\nPOST /admin/discover/m3u/search\nPOST /admin/discover/youtube/search\nPOST /admin/discover/manual/resolve"]
```

## Notes

**Middleware order matters.** `redirect_trailing_slash` is registered with `.layer()` (outermost), so it fires before route matching *and* before auth. A request to `GET /admin/` gets a 308 redirect without ever hitting the `basic_auth` middleware. Use `/admin` (no trailing slash) to test authentication.

**Auth scope.** `basic_auth` is registered with `.route_layer()` scoped to the admin sub-router only. Public routes (`/guide`, `/channel/:id/tune`, etc.) require no credentials.

**Player routes return JSON.** `/channel/:id/tune` and `/channel/:id/next` return `Json<TuneResponse>` with HTTP 200 on success or HTTP 503 on failure — they do not redirect or return HTML.

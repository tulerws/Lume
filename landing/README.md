# Lume landing page

This directory contains the standalone marketing site for Lume. It is intentionally isolated from the Tauri frontend and the Mobile PWA so visual iterations cannot affect the applications.

Build the static site with:

```bash
npm run landing:build
```

Preview it locally with:

```bash
npm run landing:dev
```

The build is written to the ignored `landing-dist/` directory. Existing project media is copied there at build time, avoiding duplicate binary assets in the repository.

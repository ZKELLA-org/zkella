# Containerized ZKELLA indexer (Tranche 2 Deliverable 4/5). Runs the same
# indexer/src/main.ts entry point as local development, against whichever
# backend DATABASE_URL points at (see indexer/src/db.ts) — in this compose
# stack, the postgres service defined in docker-compose.yml.
FROM node:22-slim

WORKDIR /app

# The indexer's dependencies live in the repo root's package.json (it's a
# source folder in this monorepo, not its own npm package — see
# indexer/README.md), so the whole workspace's manifest is what npm ci needs.
COPY package.json package-lock.json ./
COPY sdk/package.json sdk/package.json
RUN npm ci --omit=dev --omit=optional || npm install --omit=dev --omit=optional

COPY indexer ./indexer

ENV NODE_ENV=production
EXPOSE 8787

# --experimental-strip-types: the indexer's own sources are plain .ts files
# run directly, no build step — same as local development (see
# indexer/README.md's "Running" section).
CMD ["node", "--experimental-strip-types", "indexer/src/main.ts"]

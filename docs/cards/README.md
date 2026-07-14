# Generated reliability cards

Static card output is rebuilt from signed files under `public-data/` with:

```sh
receipts cards build --data public-data --out docs/cards/generated
```

The output contains static HTML, CSS, and JSON only. A clean output directory is
required so stale cards cannot survive a build. No hosted scoring service is
part of the product.

# OpenCodeReview setup

This repo uses [Alibaba's open-source `open-code-review` CLI](https://github.com/alibaba/open-code-review) (OCR) for AI-powered PR reviews. OCR is the self-hosted, OSS alternative to CodeRabbit — it reads the diff, calls a configurable LLM, and posts structured inline + summary review comments.

The workflow is defined in [`.github/workflows/open-code-review.yml`](../.github/workflows/open-code-review.yml).

## LLM provider

Reviews use **Z.AI GLM-5.2** via the [GLM Coding Plan](https://docs.z.ai/guides/overview/quick-start) endpoint:

- Endpoint: `https://api.z.ai/api/coding/paas/v4`
- Model: `glm-5.2`
- Protocol: OpenAI-compatible (`OCR_USE_ANTHROPIC=false`)

GLM Coding Plan is a flat-rate subscription; reviews do not consume pay-as-you-go token quota.

### Quota & rate limits (important)

The Coding Plan has two limits you need to know about:

1. **5-hour prompt quota** — each tier (Lite / Pro / Max) gives a fixed number of prompts per rolling 5-hour window. When exhausted, reviews fail until the next cycle. See [Z.AI FAQ](https://docs.z.ai/devpack/faq) for current numbers.
2. **Concurrency cap** — only 1-3 in-flight requests at a time (the platform adjusts dynamically, see [Usage Policy](https://docs.z.ai/devpack/usage-policy)). The workflow pins `review_concurrency: '1'` to stay under this. Triggering many PRs at once (e.g. force-re-reviewing 8 PRs in parallel) will still hit the cap because each PR runs as its own workflow.

**Practical implication:** if you force a re-review on many PRs at once, expect some to fail with `429 Too Many Requests`. Retry those individually after a few minutes.

## Triggers

| Event | Behavior |
|-------|----------|
| `pull_request_target: opened/synchronize/reopened` | Automatic review on every PR push |
| `issue_comment: created` (starts with `/open-code-review` or `@open-code-review`) | Manual re-review on demand |

The comment trigger is gated to `MEMBER`/`OWNER`/`COLLABORATOR` author association to prevent LLM-quota abuse from arbitrary commenters.

## Required secrets

One repo secret is needed (Settings → Secrets and variables → Actions → New repository secret):

| Name | Value |
|------|-------|
| `OCR_LLM_AUTH_TOKEN` | Your Z.AI API key (from <https://z.ai/model-api>, GLM Coding Plan dashboard) |

The endpoint URL and model name are pinned in the workflow file — they're not secrets, just config.

## Setting up the secret (maintainer)

1. Get a Z.AI API key from <https://z.ai/model-api> (sign up for the GLM Coding Plan if you haven't).
2. Go to **Settings → Secrets and variables → Actions → New repository secret** on GitHub.
3. Name: `OCR_LLM_AUTH_TOKEN`. Value: the API key.
4. Save. The next PR push will trigger OCR automatically.

To re-review an existing PR without pushing, comment on the PR:

```
/open-code-review
```

## Behavior knobs

These inputs to the composite action are worth knowing:

- **`sticky_summary: 'true'`** — the summary comment is updated in place on each run, rather than a new one posted. Keeps the PR thread readable across multiple pushes.
- **`incremental: 'true'`** — inline comments whose `(path, line range)` overlaps an existing bot review comment are skipped. History is never deleted (non-destructive).
- **`llm_extra_body: '{"thinking": {"type": "disabled"}}'`** — disables GLM-5.2's reasoning mode. Code review is a bounded task; we want fast responses, not chain-of-thought.
- **`upload_artifacts: 'true'`** — raw JSON + stderr are uploaded as workflow artifacts for debugging when a review looks wrong.

See [the upstream `action.yml`](https://github.com/alibaba/open-code-review/blob/main/action.yml) for the full input list.

## Comparison with CodeRabbit

| Feature | CodeRabbit | OpenCodeReview |
|---------|-----------|----------------|
| Hosting | SaaS | Self-hosted (runs in GitHub Actions) |
| Cost | Free for OSS, rate-limited | LLM cost only (GLM Coding Plan = flat rate) |
| Pre-merge check / verdict | ✅ Yes (status check) | ❌ No — review comments only, no pass/fail gate |
| Inline comments | ✅ | ✅ |
| Summary comment | ✅ | ✅ (sticky) |
| Incremental | ✅ | ✅ |
| LLM | CodeRabbit's choice | Configurable (we use GLM-5.2) |

OCR does not produce a GitHub check-run with a verdict, so PRs can be merged even if OCR finds issues. The maintainer reads the review and decides.

## Debugging

If a review run looks wrong:

1. Open the workflow run from the Actions tab.
2. Download the `ocr-review-result-*` artifact — it contains the raw LLM JSON output and stderr.
3. The most common failure is a 401/403 from Z.AI → check `OCR_LLM_AUTH_TOKEN` is set and the key is valid.
4. The next most common is a model-not-found 404 → confirm the GLM Coding Plan is active and `glm-5.2` is available on your account.

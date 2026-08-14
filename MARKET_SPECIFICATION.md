# Scarcity Market Specification v0.2

Every Hedgents market must be objectively resolvable. The market account stores cryptographic commitments to the full offchain question and rules, so neither Hedgents nor the resolver can change the contract after issuance begins.

## Required question document

- Stable market identifier
- Metal symbol and full name
- One falsifiable binary question or one objectively measurable scalar metric
- Unit of measurement
- Geographic scope
- Observation window and deadline
- Primary data source
- Named fallback sources in order
- Exact transformation and rounding rules
- Publication-delay policy
- Revision policy
- Invalid-market conditions

The canonical UTF-8 JSON is SHA-256 hashed into `question_hash` for a binary market or `metric_hash` for a curve market.

## Required rules document

- Trading open and close timestamps
- Earliest resolution timestamp
- Evidence collection procedure
- Source-conflict precedence
- Resolver key and rotation policy. The program snapshots the current resolver into the market account at creation.
- Dispute review period, when enabled
- Payout treatment for YES, NO, and invalid outcomes
- For curve markets: normalization bounds, bucket count, exact midpoint tie-break, target jackpot share, snapshotted leverage cap, and kernel version

The canonical UTF-8 JSON is SHA-256 hashed into `rules_hash`.

## Resolution report

The resolver publishes the source observations, retrieval timestamps, transformations, conclusion, and supporting links as canonical JSON. Its SHA-256 hash is committed as `resolution_report_hash` in the same transaction that resolves the market.

## Curve market rules

Curve markets accept an integer result from `-1_000_000` through `1_000_000`, representing the normalized interval `[-1, 1]`. Markets use an odd bucket count from 3 through 41 so zero always has a canonical center bucket. The program maps a result to its nearest bucket and resolves exact midpoint ties toward the higher bucket.

Each wallet has one canonical position PDA per market and bucket. Stake is held in the market's USDC vault. Users may open, add to, or withdraw from positions only while the unresolved market is open. Pausing stops new stake but never blocks withdrawals, resolution, invalid refunds, or claims.

Kernel version one uses a full-support linear accuracy weight:

```text
distance(b) = abs(b - resolved bucket)
weight(b)   = bucket count - distance(b)
```

On valid resolution, the program deducts the snapshotted protocol fee and calculates a target jackpot from the configured share. The actual exact-bucket jackpot is the smaller of that target and `exact-bucket stake × min(10, bucket count - 1)`. The remaining post-fee collateral is distributed proportional to `position stake × weight`. This prevents dust placed in every bucket from capturing a fixed jackpot; when exact stake is absent or too small, the unused target returns to the accuracy pool. All division rounds down in collateral base units. On invalid resolution, each active position receives its stake one-for-one and no fee is charged.

If the market's snapshotted resolver does not act, the protocol admin may call the recovery instruction only after seven days beyond `resolve_after`. Recovery can only invalidate the market and enable fee-free one-for-one refunds; it cannot select a numerical outcome.

The program records pool component totals and enforces `total_claimed <= payout_pool`. It calculates liabilities from recorded stake rather than the token vault balance, so unsolicited token transfers cannot enlarge payouts.

## Version-one limitations

- USDC is the sole collateral mint per deployment.
- Binary claim markets and scalar curve forecast pools only.
- Resolution uses one snapshotted signer; multisig and optimistic disputes are public-launch blockers.
- The program handles issuance, collateral, escrowed secondary limit orders, resolution, and redemption.
- Curve positions are nontransferable in version one and have no secondary orderbook.
- Filled and expired order accounts are closed by their maker through the cancel instruction; the terminal keeps those reclaim actions visible.
- No program is deployed to devnet or mainnet until integration tests and an external review are complete.

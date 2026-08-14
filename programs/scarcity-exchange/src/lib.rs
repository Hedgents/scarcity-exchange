use crate::program::ScarcityExchange;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, CloseAccount, Mint, MintTo, Token, TokenAccount, Transfer};

declare_id!("CJHWP9ed1BzWVQhUeJPQ9jJb4YcVWiFNpQcG7mPEGk86");

const CONFIG_SEED: &[u8] = b"config";
const MARKET_SEED: &[u8] = b"market";
const YES_MINT_SEED: &[u8] = b"yes_mint";
const NO_MINT_SEED: &[u8] = b"no_mint";
const VAULT_SEED: &[u8] = b"vault";
const ORDER_SEED: &[u8] = b"order";
const ORDER_VAULT_SEED: &[u8] = b"order_vault";
const CURVE_MARKET_SEED: &[u8] = b"curve_market";
const CURVE_VAULT_SEED: &[u8] = b"curve_vault";
const CURVE_POSITION_SEED: &[u8] = b"curve_position";
const PRICE_SCALE: u128 = 1_000_000;
const BPS_SCALE: u128 = 10_000;
const MAX_TRADING_FEE_BPS: u16 = 200;
const CURVE_VALUE_SCALE: i32 = 1_000_000;
const MIN_CURVE_BUCKETS: u8 = 3;
const MAX_CURVE_BUCKETS: u8 = 41;
const MAX_CURVE_JACKPOT_BPS: u16 = 5_000;
const CURVE_KERNEL_VERSION: u8 = 1;
const MAX_CURVE_JACKPOT_LEVERAGE: u8 = 10;
const CURVE_RESOLVER_RECOVERY_DELAY_SECONDS: i64 = 7 * 24 * 60 * 60;

#[program]
pub mod scarcity_exchange {
    use super::*;

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        resolver: Pubkey,
        trading_fee_bps: u16,
    ) -> Result<()> {
        require!(
            resolver != Pubkey::default(),
            ExchangeError::InvalidResolver
        );
        require!(
            trading_fee_bps <= MAX_TRADING_FEE_BPS,
            ExchangeError::FeeTooHigh
        );
        require!(
            ctx.accounts.collateral_mint.decimals == 6,
            ExchangeError::InvalidCollateralDecimals
        );

        let config = &mut ctx.accounts.config;
        config.version = 1;
        config.bump = ctx.bumps.config;
        config.paused = false;
        config.admin = ctx.accounts.admin.key();
        config.resolver = resolver;
        config.collateral_mint = ctx.accounts.collateral_mint.key();
        config.fee_recipient = ctx.accounts.fee_recipient.key();
        config.trading_fee_bps = trading_fee_bps;

        emit!(ConfigInitialized {
            admin: config.admin,
            resolver,
            collateral_mint: config.collateral_mint,
            fee_recipient: config.fee_recipient,
            trading_fee_bps,
        });

        Ok(())
    }

    pub fn set_paused(ctx: Context<AdminConfig>, paused: bool) -> Result<()> {
        ctx.accounts.config.paused = paused;
        emit!(PauseChanged { paused });
        Ok(())
    }

    pub fn set_resolver(ctx: Context<AdminConfig>, resolver: Pubkey) -> Result<()> {
        require!(
            resolver != Pubkey::default(),
            ExchangeError::InvalidResolver
        );
        let previous_resolver = ctx.accounts.config.resolver;
        ctx.accounts.config.resolver = resolver;
        emit!(ResolverChanged {
            previous_resolver,
            resolver,
        });
        Ok(())
    }

    pub fn create_market(
        ctx: Context<CreateMarket>,
        market_id: [u8; 32],
        question_hash: [u8; 32],
        rules_hash: [u8; 32],
        opens_at: i64,
        closes_at: i64,
        resolve_after: i64,
    ) -> Result<()> {
        require!(!ctx.accounts.config.paused, ExchangeError::IssuancePaused);
        require!(market_id != [0; 32], ExchangeError::InvalidMarketId);
        require!(question_hash != [0; 32], ExchangeError::InvalidCommitment);
        require!(rules_hash != [0; 32], ExchangeError::InvalidCommitment);
        require!(opens_at < closes_at, ExchangeError::InvalidSchedule);
        require!(closes_at <= resolve_after, ExchangeError::InvalidSchedule);

        let market = &mut ctx.accounts.market;
        market.version = 1;
        market.bump = ctx.bumps.market;
        market.vault_bump = ctx.bumps.vault;
        market.status = MarketStatus::Unresolved;
        market.config = ctx.accounts.config.key();
        market.creator = ctx.accounts.admin.key();
        market.resolver = ctx.accounts.config.resolver;
        market.collateral_mint = ctx.accounts.collateral_mint.key();
        market.yes_mint = ctx.accounts.yes_mint.key();
        market.no_mint = ctx.accounts.no_mint.key();
        market.vault = ctx.accounts.vault.key();
        market.market_id = market_id;
        market.question_hash = question_hash;
        market.rules_hash = rules_hash;
        market.resolution_report_hash = [0; 32];
        market.opens_at = opens_at;
        market.closes_at = closes_at;
        market.resolve_after = resolve_after;
        market.resolved_at = 0;
        market.open_interest = 0;
        market.total_redeemed = 0;

        emit!(MarketCreated {
            market: market.key(),
            market_id,
            question_hash,
            rules_hash,
            resolver: market.resolver,
            yes_mint: market.yes_mint,
            no_mint: market.no_mint,
            opens_at,
            closes_at,
            resolve_after,
        });

        Ok(())
    }

    pub fn mint_complete_set(ctx: Context<MintCompleteSet>, amount: u64) -> Result<()> {
        require!(amount > 0, ExchangeError::InvalidAmount);
        require!(!ctx.accounts.config.paused, ExchangeError::IssuancePaused);

        let now = Clock::get()?.unix_timestamp;
        let market = &ctx.accounts.market;
        require!(
            market.status == MarketStatus::Unresolved,
            ExchangeError::MarketResolved
        );
        require!(now >= market.opens_at, ExchangeError::MarketNotOpen);
        require!(now < market.closes_at, ExchangeError::MarketClosed);

        token::transfer(ctx.accounts.transfer_collateral_to_vault(), amount)?;

        let market_id = market.market_id;
        let bump = [market.bump];
        let signer_seeds: &[&[u8]] = &[MARKET_SEED, market_id.as_ref(), &bump];

        token::mint_to(
            ctx.accounts.mint_yes_to_user().with_signer(&[signer_seeds]),
            amount,
        )?;
        token::mint_to(
            ctx.accounts.mint_no_to_user().with_signer(&[signer_seeds]),
            amount,
        )?;

        let market = &mut ctx.accounts.market;
        market.open_interest = checked_add(market.open_interest, amount)?;

        emit!(CompleteSetMinted {
            market: market.key(),
            owner: ctx.accounts.owner.key(),
            amount,
            open_interest: market.open_interest,
        });

        Ok(())
    }

    pub fn merge_complete_set(ctx: Context<MergeCompleteSet>, amount: u64) -> Result<()> {
        require!(amount > 0, ExchangeError::InvalidAmount);
        require!(
            ctx.accounts.market.status == MarketStatus::Unresolved,
            ExchangeError::MarketResolved
        );
        require!(
            amount <= ctx.accounts.market.open_interest,
            ExchangeError::AmountExceedsLiability
        );

        token::burn(ctx.accounts.burn_yes(), amount)?;
        token::burn(ctx.accounts.burn_no(), amount)?;

        let market_id = ctx.accounts.market.market_id;
        let bump = [ctx.accounts.market.bump];
        let signer_seeds: &[&[u8]] = &[MARKET_SEED, market_id.as_ref(), &bump];
        token::transfer(
            ctx.accounts
                .transfer_collateral_to_owner()
                .with_signer(&[signer_seeds]),
            amount,
        )?;

        let market = &mut ctx.accounts.market;
        market.open_interest = checked_sub(market.open_interest, amount)?;

        emit!(CompleteSetMerged {
            market: market.key(),
            owner: ctx.accounts.owner.key(),
            amount,
            open_interest: market.open_interest,
        });

        Ok(())
    }

    pub fn resolve_market(
        ctx: Context<ResolveMarket>,
        outcome: ResolutionOutcome,
        resolution_report_hash: [u8; 32],
    ) -> Result<()> {
        require!(
            resolution_report_hash != [0; 32],
            ExchangeError::InvalidCommitment
        );
        require!(
            ctx.accounts.market.status == MarketStatus::Unresolved,
            ExchangeError::MarketResolved
        );

        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= ctx.accounts.market.resolve_after,
            ExchangeError::ResolutionTooEarly
        );

        let market = &mut ctx.accounts.market;
        market.status = match outcome {
            ResolutionOutcome::Yes => MarketStatus::ResolvedYes,
            ResolutionOutcome::No => MarketStatus::ResolvedNo,
            ResolutionOutcome::Invalid => MarketStatus::Invalid,
        };
        market.resolution_report_hash = resolution_report_hash;
        market.resolved_at = now;

        emit!(MarketResolved {
            market: market.key(),
            outcome,
            resolution_report_hash,
            open_interest: market.open_interest,
            resolved_at: now,
        });

        Ok(())
    }

    pub fn redeem(ctx: Context<Redeem>, amount: u64) -> Result<()> {
        require!(amount > 0, ExchangeError::InvalidAmount);

        let market = &ctx.accounts.market;
        let claim_mint = ctx.accounts.claim_mint.key();
        let payout = redemption_payout(market.status, claim_mint, market, amount)?;
        require!(payout > 0, ExchangeError::AmountTooSmall);

        let new_total = checked_add(market.total_redeemed, payout)?;
        require!(
            new_total <= market.open_interest,
            ExchangeError::AmountExceedsLiability
        );

        token::burn(ctx.accounts.burn_claim(), amount)?;

        let market_id = market.market_id;
        let bump = [market.bump];
        let signer_seeds: &[&[u8]] = &[MARKET_SEED, market_id.as_ref(), &bump];
        token::transfer(
            ctx.accounts
                .transfer_payout_to_owner()
                .with_signer(&[signer_seeds]),
            payout,
        )?;

        let market = &mut ctx.accounts.market;
        market.total_redeemed = new_total;

        emit!(PositionRedeemed {
            market: market.key(),
            owner: ctx.accounts.owner.key(),
            claim_mint,
            burned_amount: amount,
            payout,
            total_redeemed: market.total_redeemed,
        });

        Ok(())
    }

    /// Reclaim the rent of a settled market whose vault owes nothing.
    ///
    /// A market account is permanent otherwise, which at one round every fifteen minutes is the
    /// dominant running cost of the price market. Closing recovers the market account and its
    /// vault; the two outcome mints cannot be closed by the SPL token program and stay behind.
    ///
    /// The guard is that the vault has no remaining liability. `redeem` is bounded by
    /// `total_redeemed <= open_interest`, so once those are equal nobody can redeem anything ever
    /// again and the vault balance is dust-free by construction; both are checked rather than
    /// inferred. A winner who has not redeemed yet keeps the market open, which is the point:
    /// nothing here may strand a claim to reclaim rent.
    pub fn close_market(ctx: Context<CloseMarket>) -> Result<()> {
        let market = &ctx.accounts.market;
        require!(
            market.status != MarketStatus::Unresolved,
            ExchangeError::MarketNotResolved
        );
        require!(
            market.total_redeemed == market.open_interest,
            ExchangeError::AmountExceedsLiability
        );
        require!(
            ctx.accounts.vault.amount == 0,
            ExchangeError::AmountExceedsLiability
        );

        let market_id = market.market_id;
        let bump = [market.bump];
        let signer_seeds: &[&[u8]] = &[MARKET_SEED, market_id.as_ref(), &bump];
        token::close_account(ctx.accounts.close_vault().with_signer(&[signer_seeds]))?;

        emit!(MarketClosed {
            market: ctx.accounts.market.key(),
            market_id,
            open_interest: ctx.accounts.market.open_interest,
            total_redeemed: ctx.accounts.market.total_redeemed,
        });
        Ok(())
    }

    pub fn create_curve_market(
        ctx: Context<CreateCurveMarket>,
        market_id: [u8; 32],
        metric_hash: [u8; 32],
        rules_hash: [u8; 32],
        opens_at: i64,
        closes_at: i64,
        resolve_after: i64,
        bucket_count: u8,
        jackpot_bps: u16,
    ) -> Result<()> {
        require!(!ctx.accounts.config.paused, ExchangeError::IssuancePaused);
        require!(market_id != [0; 32], ExchangeError::InvalidMarketId);
        require!(metric_hash != [0; 32], ExchangeError::InvalidCommitment);
        require!(rules_hash != [0; 32], ExchangeError::InvalidCommitment);
        require!(opens_at < closes_at, ExchangeError::InvalidSchedule);
        require!(closes_at <= resolve_after, ExchangeError::InvalidSchedule);
        let _ = curve_recovery_at(resolve_after)?;
        validate_curve_parameters(bucket_count, jackpot_bps)?;

        let market = &mut ctx.accounts.curve_market;
        market.version = 1;
        market.bump = ctx.bumps.curve_market;
        market.vault_bump = ctx.bumps.curve_vault;
        market.status = CurveMarketStatus::Unresolved;
        market.kernel_version = CURVE_KERNEL_VERSION;
        market.jackpot_leverage_cap = curve_jackpot_leverage_cap(bucket_count)?;
        market.config = ctx.accounts.config.key();
        market.creator = ctx.accounts.admin.key();
        market.resolver = ctx.accounts.config.resolver;
        market.collateral_mint = ctx.accounts.collateral_mint.key();
        market.vault = ctx.accounts.curve_vault.key();
        market.fee_recipient = ctx.accounts.fee_recipient.key();
        market.market_id = market_id;
        market.metric_hash = metric_hash;
        market.rules_hash = rules_hash;
        market.resolution_report_hash = [0; 32];
        market.opens_at = opens_at;
        market.closes_at = closes_at;
        market.resolve_after = resolve_after;
        market.resolved_at = 0;
        market.normalized_outcome = 0;
        market.bucket_count = bucket_count;
        market.winning_bucket = u8::MAX;
        market.jackpot_bps = jackpot_bps;
        market.fee_bps = ctx.accounts.config.trading_fee_bps;
        market.total_staked = 0;
        market.protocol_fee = 0;
        market.payout_pool = 0;
        market.jackpot_pool = 0;
        market.curve_pool = 0;
        market.exact_stake = 0;
        market.weighted_stake = 0;
        market.total_claimed = 0;
        market.bucket_stakes = [0; MAX_CURVE_BUCKETS as usize];

        emit!(CurveMarketCreated {
            market: market.key(),
            market_id,
            metric_hash,
            rules_hash,
            resolver: market.resolver,
            collateral_mint: market.collateral_mint,
            bucket_count,
            jackpot_bps,
            fee_bps: market.fee_bps,
            kernel_version: market.kernel_version,
            jackpot_leverage_cap: market.jackpot_leverage_cap,
            opens_at,
            closes_at,
            resolve_after,
        });

        Ok(())
    }

    pub fn open_curve_position(
        ctx: Context<OpenCurvePosition>,
        bucket: u8,
        amount: u64,
    ) -> Result<()> {
        require!(amount > 0, ExchangeError::InvalidAmount);
        require!(!ctx.accounts.config.paused, ExchangeError::IssuancePaused);
        validate_curve_staking_window(&ctx.accounts.curve_market)?;
        require!(
            bucket < ctx.accounts.curve_market.bucket_count,
            ExchangeError::InvalidCurveBucket
        );

        let market = &ctx.accounts.curve_market;
        let next_total = checked_add(market.total_staked, amount)?;
        require!(
            next_total <= max_curve_total_stake(market.bucket_count)?,
            ExchangeError::CurveStakeLimitExceeded
        );
        let bucket_index = usize::from(bucket);
        let next_bucket_stake = checked_add(market.bucket_stakes[bucket_index], amount)?;

        token::transfer(ctx.accounts.transfer_curve_stake_to_vault(), amount)?;

        let position = &mut ctx.accounts.position;
        position.version = 1;
        position.bump = ctx.bumps.position;
        position.market = ctx.accounts.curve_market.key();
        position.owner = ctx.accounts.owner.key();
        position.bucket = bucket;
        position.claimed = false;
        position.stake = amount;
        position.payout = 0;

        let market = &mut ctx.accounts.curve_market;
        market.total_staked = next_total;
        market.bucket_stakes[bucket_index] = next_bucket_stake;

        emit!(CurvePositionOpened {
            market: market.key(),
            position: position.key(),
            owner: position.owner,
            bucket,
            amount,
            position_stake: position.stake,
            total_staked: market.total_staked,
        });

        Ok(())
    }

    pub fn add_curve_stake(ctx: Context<UpdateCurveStake>, amount: u64) -> Result<()> {
        require!(amount > 0, ExchangeError::InvalidAmount);
        require!(!ctx.accounts.config.paused, ExchangeError::IssuancePaused);
        require!(
            !ctx.accounts.position.claimed,
            ExchangeError::PositionAlreadyClaimed
        );
        validate_curve_staking_window(&ctx.accounts.curve_market)?;

        let market = &ctx.accounts.curve_market;
        let bucket_index = usize::from(ctx.accounts.position.bucket);
        require!(
            bucket_index < usize::from(market.bucket_count),
            ExchangeError::InvalidCurveBucket
        );
        let next_total = checked_add(market.total_staked, amount)?;
        require!(
            next_total <= max_curve_total_stake(market.bucket_count)?,
            ExchangeError::CurveStakeLimitExceeded
        );
        let next_bucket_stake = checked_add(market.bucket_stakes[bucket_index], amount)?;
        let next_position_stake = checked_add(ctx.accounts.position.stake, amount)?;

        token::transfer(ctx.accounts.transfer_curve_stake_to_vault(), amount)?;

        let position = &mut ctx.accounts.position;
        position.stake = next_position_stake;
        let market = &mut ctx.accounts.curve_market;
        market.total_staked = next_total;
        market.bucket_stakes[bucket_index] = next_bucket_stake;

        emit!(CurveStakeAdded {
            market: market.key(),
            position: position.key(),
            owner: position.owner,
            bucket: position.bucket,
            amount,
            position_stake: position.stake,
            total_staked: market.total_staked,
        });

        Ok(())
    }

    pub fn withdraw_curve_stake(ctx: Context<UpdateCurveStake>, amount: u64) -> Result<()> {
        require!(amount > 0, ExchangeError::InvalidAmount);
        require!(
            !ctx.accounts.position.claimed,
            ExchangeError::PositionAlreadyClaimed
        );
        validate_curve_staking_window(&ctx.accounts.curve_market)?;
        require!(
            amount <= ctx.accounts.position.stake,
            ExchangeError::AmountExceedsPosition
        );

        let bucket_index = usize::from(ctx.accounts.position.bucket);
        require!(
            bucket_index < usize::from(ctx.accounts.curve_market.bucket_count),
            ExchangeError::InvalidCurveBucket
        );
        let next_position_stake = checked_sub(ctx.accounts.position.stake, amount)?;
        let next_total = checked_sub(ctx.accounts.curve_market.total_staked, amount)?;
        let next_bucket_stake = checked_sub(
            ctx.accounts.curve_market.bucket_stakes[bucket_index],
            amount,
        )?;

        let market_id = ctx.accounts.curve_market.market_id;
        let bump = [ctx.accounts.curve_market.bump];
        let signer_seeds: &[&[u8]] = &[CURVE_MARKET_SEED, market_id.as_ref(), &bump];
        token::transfer(
            ctx.accounts
                .transfer_curve_stake_to_owner()
                .with_signer(&[signer_seeds]),
            amount,
        )?;

        let position = &mut ctx.accounts.position;
        position.stake = next_position_stake;
        let market = &mut ctx.accounts.curve_market;
        market.total_staked = next_total;
        market.bucket_stakes[bucket_index] = next_bucket_stake;

        emit!(CurveStakeWithdrawn {
            market: market.key(),
            position: position.key(),
            owner: position.owner,
            bucket: position.bucket,
            amount,
            position_stake: position.stake,
            total_staked: market.total_staked,
        });

        Ok(())
    }

    pub fn resolve_curve_market(
        ctx: Context<ResolveCurveMarket>,
        normalized_outcome: i32,
        resolution_report_hash: [u8; 32],
    ) -> Result<()> {
        require!(
            resolution_report_hash != [0; 32],
            ExchangeError::InvalidCommitment
        );
        require!(
            ctx.accounts.curve_market.status == CurveMarketStatus::Unresolved,
            ExchangeError::MarketResolved
        );
        require!(
            (-CURVE_VALUE_SCALE..=CURVE_VALUE_SCALE).contains(&normalized_outcome),
            ExchangeError::InvalidCurveValue
        );
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= ctx.accounts.curve_market.resolve_after,
            ExchangeError::ResolutionTooEarly
        );
        require!(
            ctx.accounts.curve_vault.amount >= ctx.accounts.curve_market.total_staked,
            ExchangeError::InsufficientCurveCollateral
        );

        let market = &ctx.accounts.curve_market;
        require!(
            curve_recorded_stake_total(&market.bucket_stakes, market.bucket_count)?
                == market.total_staked,
            ExchangeError::InvalidCurveAccounting
        );
        let winning_bucket = curve_bucket_for_value(normalized_outcome, market.bucket_count)?;
        let exact_stake = market.bucket_stakes[usize::from(winning_bucket)];
        let weighted_stake =
            curve_weighted_total(&market.bucket_stakes, market.bucket_count, winning_bucket)?;
        require!(
            market.total_staked == 0 || weighted_stake > 0,
            ExchangeError::InvalidCurveDenominator
        );
        let protocol_fee = floor_mul_div_u64(
            market.total_staked,
            u64::from(market.fee_bps),
            BPS_SCALE as u64,
        )?;
        let payout_pool = checked_sub(market.total_staked, protocol_fee)?;
        require!(
            market.jackpot_leverage_cap > 0
                && market.jackpot_leverage_cap < market.bucket_count
                && market.jackpot_leverage_cap <= MAX_CURVE_JACKPOT_LEVERAGE,
            ExchangeError::InvalidCurveAccounting
        );
        let target_jackpot =
            floor_mul_div_u64(payout_pool, u64::from(market.jackpot_bps), BPS_SCALE as u64)?;
        let jackpot_cap = exact_stake
            .checked_mul(u64::from(market.jackpot_leverage_cap))
            .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?;
        let jackpot_pool = target_jackpot.min(jackpot_cap);
        let curve_pool = checked_sub(payout_pool, jackpot_pool)?;

        if protocol_fee > 0 {
            let market_id = market.market_id;
            let bump = [market.bump];
            let signer_seeds: &[&[u8]] = &[CURVE_MARKET_SEED, market_id.as_ref(), &bump];
            token::transfer(
                ctx.accounts
                    .transfer_curve_fee()
                    .with_signer(&[signer_seeds]),
                protocol_fee,
            )?;
        }

        let market = &mut ctx.accounts.curve_market;
        market.status = CurveMarketStatus::Resolved;
        market.resolution_report_hash = resolution_report_hash;
        market.resolved_at = now;
        market.normalized_outcome = normalized_outcome;
        market.winning_bucket = winning_bucket;
        market.protocol_fee = protocol_fee;
        market.payout_pool = payout_pool;
        market.jackpot_pool = jackpot_pool;
        market.curve_pool = curve_pool;
        market.exact_stake = exact_stake;
        market.weighted_stake = weighted_stake;

        emit!(CurveMarketResolved {
            market: market.key(),
            normalized_outcome,
            winning_bucket,
            resolution_report_hash,
            total_staked: market.total_staked,
            protocol_fee,
            payout_pool,
            jackpot_pool,
            curve_pool,
            exact_stake,
            weighted_stake,
            resolved_at: now,
        });

        Ok(())
    }

    pub fn invalidate_curve_market(
        ctx: Context<InvalidateCurveMarket>,
        resolution_report_hash: [u8; 32],
    ) -> Result<()> {
        require!(
            resolution_report_hash != [0; 32],
            ExchangeError::InvalidCommitment
        );
        require!(
            ctx.accounts.curve_market.status == CurveMarketStatus::Unresolved,
            ExchangeError::MarketResolved
        );
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= ctx.accounts.curve_market.resolve_after,
            ExchangeError::ResolutionTooEarly
        );
        require!(
            ctx.accounts.curve_vault.amount >= ctx.accounts.curve_market.total_staked,
            ExchangeError::InsufficientCurveCollateral
        );
        require!(
            curve_recorded_stake_total(
                &ctx.accounts.curve_market.bucket_stakes,
                ctx.accounts.curve_market.bucket_count,
            )? == ctx.accounts.curve_market.total_staked,
            ExchangeError::InvalidCurveAccounting
        );

        let market = &mut ctx.accounts.curve_market;
        market.status = CurveMarketStatus::Invalid;
        market.resolution_report_hash = resolution_report_hash;
        market.resolved_at = now;
        market.normalized_outcome = 0;
        market.winning_bucket = u8::MAX;
        market.protocol_fee = 0;
        market.payout_pool = market.total_staked;
        market.jackpot_pool = 0;
        market.curve_pool = market.total_staked;
        market.exact_stake = 0;
        market.weighted_stake = 0;

        emit!(CurveMarketInvalidated {
            market: market.key(),
            resolution_report_hash,
            total_staked: market.total_staked,
            resolved_at: now,
        });

        Ok(())
    }

    pub fn recover_curve_market(
        ctx: Context<RecoverCurveMarket>,
        resolution_report_hash: [u8; 32],
    ) -> Result<()> {
        require!(
            resolution_report_hash != [0; 32],
            ExchangeError::InvalidCommitment
        );
        require!(
            ctx.accounts.curve_market.status == CurveMarketStatus::Unresolved,
            ExchangeError::MarketResolved
        );
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= curve_recovery_at(ctx.accounts.curve_market.resolve_after)?,
            ExchangeError::CurveRecoveryTooEarly
        );
        require!(
            ctx.accounts.curve_vault.amount >= ctx.accounts.curve_market.total_staked,
            ExchangeError::InsufficientCurveCollateral
        );
        require!(
            curve_recorded_stake_total(
                &ctx.accounts.curve_market.bucket_stakes,
                ctx.accounts.curve_market.bucket_count,
            )? == ctx.accounts.curve_market.total_staked,
            ExchangeError::InvalidCurveAccounting
        );

        let market = &mut ctx.accounts.curve_market;
        market.status = CurveMarketStatus::Invalid;
        market.resolution_report_hash = resolution_report_hash;
        market.resolved_at = now;
        market.normalized_outcome = 0;
        market.winning_bucket = u8::MAX;
        market.protocol_fee = 0;
        market.payout_pool = market.total_staked;
        market.jackpot_pool = 0;
        market.curve_pool = market.total_staked;
        market.exact_stake = 0;
        market.weighted_stake = 0;

        emit!(CurveMarketRecovered {
            market: market.key(),
            admin: ctx.accounts.admin.key(),
            resolution_report_hash,
            total_staked: market.total_staked,
            recovered_at: now,
        });

        Ok(())
    }

    pub fn claim_curve_position(ctx: Context<ClaimCurvePosition>) -> Result<()> {
        require!(
            ctx.accounts.curve_market.status != CurveMarketStatus::Unresolved,
            ExchangeError::MarketNotResolved
        );
        require!(
            !ctx.accounts.position.claimed,
            ExchangeError::PositionAlreadyClaimed
        );

        let payout = curve_position_payout(&ctx.accounts.curve_market, &ctx.accounts.position)?;
        let new_total_claimed = checked_add(ctx.accounts.curve_market.total_claimed, payout)?;
        require!(
            new_total_claimed <= ctx.accounts.curve_market.payout_pool,
            ExchangeError::AmountExceedsLiability
        );

        if payout > 0 {
            let market_id = ctx.accounts.curve_market.market_id;
            let bump = [ctx.accounts.curve_market.bump];
            let signer_seeds: &[&[u8]] = &[CURVE_MARKET_SEED, market_id.as_ref(), &bump];
            token::transfer(
                ctx.accounts
                    .transfer_curve_payout()
                    .with_signer(&[signer_seeds]),
                payout,
            )?;
        }

        let position = &mut ctx.accounts.position;
        position.claimed = true;
        position.payout = payout;
        let market = &mut ctx.accounts.curve_market;
        market.total_claimed = new_total_claimed;

        emit!(CurvePositionClaimed {
            market: market.key(),
            position: position.key(),
            owner: position.owner,
            bucket: position.bucket,
            stake: position.stake,
            payout,
            total_claimed: market.total_claimed,
        });

        Ok(())
    }

    pub fn place_order(
        ctx: Context<PlaceOrder>,
        order_id: [u8; 32],
        side: OrderSide,
        price_micro_usdc: u64,
        quantity: u64,
        expires_at: i64,
    ) -> Result<()> {
        require!(order_id != [0; 32], ExchangeError::InvalidOrderId);
        require!(quantity > 0, ExchangeError::InvalidAmount);
        require!(
            price_micro_usdc > 0 && u128::from(price_micro_usdc) <= PRICE_SCALE,
            ExchangeError::InvalidPrice
        );
        require!(!ctx.accounts.config.paused, ExchangeError::IssuancePaused);
        require!(
            ctx.accounts.market.status == MarketStatus::Unresolved,
            ExchangeError::MarketResolved
        );

        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= ctx.accounts.market.opens_at,
            ExchangeError::MarketNotOpen
        );
        require!(
            now < ctx.accounts.market.closes_at,
            ExchangeError::MarketClosed
        );
        require!(expires_at > now, ExchangeError::OrderExpired);
        require!(
            expires_at <= ctx.accounts.market.closes_at,
            ExchangeError::InvalidSchedule
        );

        let outcome_mint = ctx.accounts.outcome_mint.key();
        require!(
            outcome_mint == ctx.accounts.market.yes_mint
                || outcome_mint == ctx.accounts.market.no_mint,
            ExchangeError::WrongOutcomeMint
        );
        let expected_escrow_mint = match side {
            OrderSide::Bid => ctx.accounts.collateral_mint.key(),
            OrderSide::Ask => outcome_mint,
        };
        require_keys_eq!(
            ctx.accounts.escrow_mint.key(),
            expected_escrow_mint,
            ExchangeError::WrongEscrowMint
        );
        require_keys_eq!(
            ctx.accounts.maker_source.mint,
            expected_escrow_mint,
            ExchangeError::WrongEscrowMint
        );

        let full_quote = quote_for_quantity(quantity, price_micro_usdc)?;
        let full_fee = fee_for_quote(full_quote, ctx.accounts.config.trading_fee_bps)?;
        let escrow_amount = match side {
            OrderSide::Bid => checked_add(full_quote, full_fee)?,
            OrderSide::Ask => quantity,
        };
        token::transfer(ctx.accounts.transfer_to_order_vault(), escrow_amount)?;

        let order = &mut ctx.accounts.order;
        order.version = 1;
        order.bump = ctx.bumps.order;
        order.vault_bump = ctx.bumps.order_vault;
        order.side = side;
        order.market = ctx.accounts.market.key();
        order.maker = ctx.accounts.maker.key();
        order.collateral_mint = ctx.accounts.collateral_mint.key();
        order.outcome_mint = outcome_mint;
        order.escrow_mint = expected_escrow_mint;
        order.vault = ctx.accounts.order_vault.key();
        order.fee_recipient = ctx.accounts.fee_recipient.key();
        order.order_id = order_id;
        order.price_micro_usdc = price_micro_usdc;
        order.original_quantity = quantity;
        order.remaining_quantity = quantity;
        order.quote_filled = 0;
        order.fee_paid = 0;
        order.fee_bps = ctx.accounts.config.trading_fee_bps;
        order.expires_at = expires_at;

        emit!(OrderPlaced {
            order: order.key(),
            market: order.market,
            maker: order.maker,
            side,
            outcome_mint,
            price_micro_usdc,
            quantity,
            escrow_amount,
            fee_bps: order.fee_bps,
            expires_at,
        });

        Ok(())
    }

    pub fn fill_ask(ctx: Context<FillAsk>, quantity: u64) -> Result<()> {
        validate_fill(
            &ctx.accounts.market,
            &ctx.accounts.order,
            OrderSide::Ask,
            quantity,
        )?;
        let (quote_delta, fee_delta, quote_after, fee_after) =
            fill_amounts(&ctx.accounts.order, quantity)?;

        token::transfer(ctx.accounts.transfer_quote_to_maker(), quote_delta)?;
        if fee_delta > 0 {
            token::transfer(ctx.accounts.transfer_fee_from_taker(), fee_delta)?;
        }

        let market_key = ctx.accounts.market.key();
        let maker = ctx.accounts.order.maker;
        let order_id = ctx.accounts.order.order_id;
        let bump = [ctx.accounts.order.bump];
        let signer_seeds: &[&[u8]] = &[
            ORDER_SEED,
            market_key.as_ref(),
            maker.as_ref(),
            order_id.as_ref(),
            &bump,
        ];
        token::transfer(
            ctx.accounts
                .transfer_outcome_to_taker()
                .with_signer(&[signer_seeds]),
            quantity,
        )?;

        let order = &mut ctx.accounts.order;
        order.remaining_quantity = checked_sub(order.remaining_quantity, quantity)?;
        order.quote_filled = quote_after;
        order.fee_paid = fee_after;

        emit!(OrderFilled {
            order: order.key(),
            market: order.market,
            maker: order.maker,
            taker: ctx.accounts.taker.key(),
            side: order.side,
            quantity,
            quote_amount: quote_delta,
            fee_amount: fee_delta,
            remaining_quantity: order.remaining_quantity,
        });

        Ok(())
    }

    pub fn fill_bid(ctx: Context<FillBid>, quantity: u64) -> Result<()> {
        validate_fill(
            &ctx.accounts.market,
            &ctx.accounts.order,
            OrderSide::Bid,
            quantity,
        )?;
        let (quote_delta, fee_delta, quote_after, fee_after) =
            fill_amounts(&ctx.accounts.order, quantity)?;

        token::transfer(ctx.accounts.transfer_outcome_to_maker(), quantity)?;

        let market_key = ctx.accounts.market.key();
        let maker = ctx.accounts.order.maker;
        let order_id = ctx.accounts.order.order_id;
        let bump = [ctx.accounts.order.bump];
        let signer_seeds: &[&[u8]] = &[
            ORDER_SEED,
            market_key.as_ref(),
            maker.as_ref(),
            order_id.as_ref(),
            &bump,
        ];
        token::transfer(
            ctx.accounts
                .transfer_quote_to_taker()
                .with_signer(&[signer_seeds]),
            quote_delta,
        )?;
        if fee_delta > 0 {
            token::transfer(
                ctx.accounts
                    .transfer_fee_from_vault()
                    .with_signer(&[signer_seeds]),
                fee_delta,
            )?;
        }

        let order = &mut ctx.accounts.order;
        order.remaining_quantity = checked_sub(order.remaining_quantity, quantity)?;
        order.quote_filled = quote_after;
        order.fee_paid = fee_after;

        emit!(OrderFilled {
            order: order.key(),
            market: order.market,
            maker: order.maker,
            taker: ctx.accounts.taker.key(),
            side: order.side,
            quantity,
            quote_amount: quote_delta,
            fee_amount: fee_delta,
            remaining_quantity: order.remaining_quantity,
        });

        Ok(())
    }

    pub fn cancel_order(ctx: Context<CancelOrder>) -> Result<()> {
        let refund_amount = ctx.accounts.vault.amount;
        let market_key = ctx.accounts.market.key();
        let maker = ctx.accounts.order.maker;
        let order_id = ctx.accounts.order.order_id;
        let bump = [ctx.accounts.order.bump];
        let signer_seeds: &[&[u8]] = &[
            ORDER_SEED,
            market_key.as_ref(),
            maker.as_ref(),
            order_id.as_ref(),
            &bump,
        ];

        if refund_amount > 0 {
            token::transfer(
                ctx.accounts
                    .transfer_refund_to_maker()
                    .with_signer(&[signer_seeds]),
                refund_amount,
            )?;
        }
        token::close_account(
            ctx.accounts
                .close_order_vault()
                .with_signer(&[signer_seeds]),
        )?;

        emit!(OrderCancelled {
            order: ctx.accounts.order.key(),
            market: ctx.accounts.order.market,
            maker,
            refund_amount,
            unfilled_quantity: ctx.accounts.order.remaining_quantity,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + ExchangeConfig::INIT_SPACE,
        seeds = [CONFIG_SEED],
        bump
    )]
    pub config: Account<'info, ExchangeConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        constraint = program.programdata_address()? == Some(program_data.key()) @ ExchangeError::Unauthorized
    )]
    pub program: Program<'info, ScarcityExchange>,
    #[account(
        constraint = program_data.upgrade_authority_address == Some(admin.key()) @ ExchangeError::Unauthorized
    )]
    pub program_data: Account<'info, ProgramData>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        constraint = fee_recipient.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub fee_recipient: Box<Account<'info, TokenAccount>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AdminConfig<'info> {
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = admin @ ExchangeError::Unauthorized
    )]
    pub config: Account<'info, ExchangeConfig>,
    pub admin: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(market_id: [u8; 32])]
pub struct CreateMarket<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = admin @ ExchangeError::Unauthorized,
        has_one = collateral_mint @ ExchangeError::WrongCollateralMint
    )]
    pub config: Account<'info, ExchangeConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub collateral_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = admin,
        space = 8 + ScarcityMarket::INIT_SPACE,
        seeds = [MARKET_SEED, market_id.as_ref()],
        bump
    )]
    pub market: Account<'info, ScarcityMarket>,
    #[account(
        init,
        payer = admin,
        seeds = [YES_MINT_SEED, market.key().as_ref()],
        bump,
        mint::decimals = collateral_mint.decimals,
        mint::authority = market
    )]
    pub yes_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = admin,
        seeds = [NO_MINT_SEED, market.key().as_ref()],
        bump,
        mint::decimals = collateral_mint.decimals,
        mint::authority = market
    )]
    pub no_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = admin,
        seeds = [VAULT_SEED, market.key().as_ref()],
        bump,
        token::mint = collateral_mint,
        token::authority = market
    )]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct MintCompleteSet<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    #[account(
        mut,
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = config,
        has_one = collateral_mint,
        has_one = yes_mint,
        has_one = no_mint,
        has_one = vault
    )]
    pub market: Box<Account<'info, ScarcityMarket>>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = owner_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint,
        constraint = owner_collateral.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_collateral: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub yes_mint: Box<Account<'info, Mint>>,
    #[account(mut)]
    pub no_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = owner_yes.mint == yes_mint.key() @ ExchangeError::WrongOutcomeMint,
        constraint = owner_yes.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_yes: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = owner_no.mint == no_mint.key() @ ExchangeError::WrongOutcomeMint,
        constraint = owner_no.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_no: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> MintCompleteSet<'info> {
    fn transfer_collateral_to_vault(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.owner_collateral.to_account_info(),
                to: self.vault.to_account_info(),
                authority: self.owner.to_account_info(),
            },
        )
    }

    fn mint_yes_to_user(&self) -> CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            MintTo {
                mint: self.yes_mint.to_account_info(),
                to: self.owner_yes.to_account_info(),
                authority: self.market.to_account_info(),
            },
        )
    }

    fn mint_no_to_user(&self) -> CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            MintTo {
                mint: self.no_mint.to_account_info(),
                to: self.owner_no.to_account_info(),
                authority: self.market.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct MergeCompleteSet<'info> {
    #[account(
        mut,
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = collateral_mint,
        has_one = yes_mint,
        has_one = no_mint,
        has_one = vault
    )]
    pub market: Box<Account<'info, ScarcityMarket>>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = owner_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint,
        constraint = owner_collateral.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_collateral: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub yes_mint: Box<Account<'info, Mint>>,
    #[account(mut)]
    pub no_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = owner_yes.mint == yes_mint.key() @ ExchangeError::WrongOutcomeMint,
        constraint = owner_yes.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_yes: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = owner_no.mint == no_mint.key() @ ExchangeError::WrongOutcomeMint,
        constraint = owner_no.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_no: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> MergeCompleteSet<'info> {
    fn burn_yes(&self) -> CpiContext<'_, '_, '_, 'info, Burn<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Burn {
                mint: self.yes_mint.to_account_info(),
                from: self.owner_yes.to_account_info(),
                authority: self.owner.to_account_info(),
            },
        )
    }

    fn burn_no(&self) -> CpiContext<'_, '_, '_, 'info, Burn<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Burn {
                mint: self.no_mint.to_account_info(),
                from: self.owner_no.to_account_info(),
                authority: self.owner.to_account_info(),
            },
        )
    }

    fn transfer_collateral_to_owner(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.vault.to_account_info(),
                to: self.owner_collateral.to_account_info(),
                authority: self.market.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct ResolveMarket<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Account<'info, ExchangeConfig>,
    pub resolver: Signer<'info>,
    #[account(
        mut,
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = config,
        has_one = resolver @ ExchangeError::Unauthorized
    )]
    pub market: Account<'info, ScarcityMarket>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(
        mut,
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = collateral_mint,
        has_one = vault
    )]
    pub market: Account<'info, ScarcityMarket>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub collateral_mint: Account<'info, Mint>,
    #[account(
        mut,
        constraint = owner_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint,
        constraint = owner_collateral.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_collateral: Account<'info, TokenAccount>,
    #[account(mut)]
    pub claim_mint: Account<'info, Mint>,
    #[account(
        mut,
        constraint = owner_claim.mint == claim_mint.key() @ ExchangeError::WrongOutcomeMint,
        constraint = owner_claim.owner == owner.key() @ ExchangeError::Unauthorized
    )]
    pub owner_claim: Account<'info, TokenAccount>,
    #[account(mut)]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CloseMarket<'info> {
    #[account(has_one = admin @ ExchangeError::Unauthorized)]
    pub config: Account<'info, ExchangeConfig>,
    /// Receives the reclaimed rent, and is the only party allowed to reclaim it.
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        mut,
        close = admin,
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = config,
        has_one = vault
    )]
    pub market: Account<'info, ScarcityMarket>,
    #[account(mut)]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

impl<'info> CloseMarket<'info> {
    fn close_vault(&self) -> CpiContext<'_, '_, '_, 'info, CloseAccount<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            CloseAccount {
                account: self.vault.to_account_info(),
                destination: self.admin.to_account_info(),
                authority: self.market.to_account_info(),
            },
        )
    }
}

impl<'info> Redeem<'info> {
    fn burn_claim(&self) -> CpiContext<'_, '_, '_, 'info, Burn<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Burn {
                mint: self.claim_mint.to_account_info(),
                from: self.owner_claim.to_account_info(),
                authority: self.owner.to_account_info(),
            },
        )
    }

    fn transfer_payout_to_owner(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.vault.to_account_info(),
                to: self.owner_collateral.to_account_info(),
                authority: self.market.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
#[instruction(market_id: [u8; 32])]
pub struct CreateCurveMarket<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = admin @ ExchangeError::Unauthorized,
        has_one = collateral_mint @ ExchangeError::WrongCollateralMint,
        has_one = fee_recipient @ ExchangeError::WrongFeeRecipient
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        constraint = fee_recipient.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub fee_recipient: Box<Account<'info, TokenAccount>>,
    #[account(
        init,
        payer = admin,
        space = 8 + CurveMarket::INIT_SPACE,
        seeds = [CURVE_MARKET_SEED, market_id.as_ref()],
        bump
    )]
    pub curve_market: Box<Account<'info, CurveMarket>>,
    #[account(
        init,
        payer = admin,
        seeds = [CURVE_VAULT_SEED, curve_market.key().as_ref()],
        bump,
        token::mint = collateral_mint,
        token::authority = curve_market
    )]
    pub curve_vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(bucket: u8)]
pub struct OpenCurvePosition<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = collateral_mint @ ExchangeError::WrongCollateralMint
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    #[account(
        mut,
        seeds = [CURVE_MARKET_SEED, curve_market.market_id.as_ref()],
        bump = curve_market.bump,
        has_one = config,
        has_one = collateral_mint,
        constraint = curve_market.vault == curve_vault.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_market: Box<Account<'info, CurveMarket>>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = owner_collateral.owner == owner.key() @ ExchangeError::Unauthorized,
        constraint = owner_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub owner_collateral: Box<Account<'info, TokenAccount>>,
    #[account(
        init,
        payer = owner,
        space = 8 + CurvePosition::INIT_SPACE,
        seeds = [
            CURVE_POSITION_SEED,
            curve_market.key().as_ref(),
            owner.key().as_ref(),
            &[bucket]
        ],
        bump
    )]
    pub position: Box<Account<'info, CurvePosition>>,
    #[account(
        mut,
        constraint = curve_vault.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint,
        constraint = curve_vault.owner == curve_market.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

impl<'info> OpenCurvePosition<'info> {
    fn transfer_curve_stake_to_vault(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.owner_collateral.to_account_info(),
                to: self.curve_vault.to_account_info(),
                authority: self.owner.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct UpdateCurveStake<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = collateral_mint @ ExchangeError::WrongCollateralMint
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    #[account(
        mut,
        seeds = [CURVE_MARKET_SEED, curve_market.market_id.as_ref()],
        bump = curve_market.bump,
        has_one = config,
        has_one = collateral_mint,
        constraint = curve_market.vault == curve_vault.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_market: Box<Account<'info, CurveMarket>>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = owner_collateral.owner == owner.key() @ ExchangeError::Unauthorized,
        constraint = owner_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub owner_collateral: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [
            CURVE_POSITION_SEED,
            curve_market.key().as_ref(),
            owner.key().as_ref(),
            &[position.bucket]
        ],
        bump = position.bump,
        constraint = position.market == curve_market.key() @ ExchangeError::WrongCurveMarket,
        has_one = owner @ ExchangeError::Unauthorized
    )]
    pub position: Box<Account<'info, CurvePosition>>,
    #[account(
        mut,
        constraint = curve_vault.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint,
        constraint = curve_vault.owner == curve_market.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> UpdateCurveStake<'info> {
    fn transfer_curve_stake_to_vault(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.owner_collateral.to_account_info(),
                to: self.curve_vault.to_account_info(),
                authority: self.owner.to_account_info(),
            },
        )
    }

    fn transfer_curve_stake_to_owner(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.curve_vault.to_account_info(),
                to: self.owner_collateral.to_account_info(),
                authority: self.curve_market.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct ResolveCurveMarket<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    pub resolver: Signer<'info>,
    #[account(
        mut,
        seeds = [CURVE_MARKET_SEED, curve_market.market_id.as_ref()],
        bump = curve_market.bump,
        has_one = config,
        has_one = resolver @ ExchangeError::Unauthorized,
        has_one = collateral_mint @ ExchangeError::WrongCollateralMint,
        has_one = fee_recipient @ ExchangeError::WrongFeeRecipient,
        constraint = curve_market.vault == curve_vault.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_market: Box<Account<'info, CurveMarket>>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = curve_vault.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint,
        constraint = curve_vault.owner == curve_market.key() @ ExchangeError::WrongCurveVault,
        constraint = curve_vault.key() != fee_recipient.key() @ ExchangeError::DuplicateAccount
    )]
    pub curve_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = fee_recipient.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub fee_recipient: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> ResolveCurveMarket<'info> {
    fn transfer_curve_fee(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.curve_vault.to_account_info(),
                to: self.fee_recipient.to_account_info(),
                authority: self.curve_market.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct InvalidateCurveMarket<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    pub resolver: Signer<'info>,
    #[account(
        mut,
        seeds = [CURVE_MARKET_SEED, curve_market.market_id.as_ref()],
        bump = curve_market.bump,
        has_one = config,
        has_one = resolver @ ExchangeError::Unauthorized,
        constraint = curve_market.vault == curve_vault.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_market: Box<Account<'info, CurveMarket>>,
    #[account(
        constraint = curve_vault.mint == curve_market.collateral_mint @ ExchangeError::WrongCollateralMint,
        constraint = curve_vault.owner == curve_market.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_vault: Box<Account<'info, TokenAccount>>,
}

#[derive(Accounts)]
pub struct RecoverCurveMarket<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = admin @ ExchangeError::Unauthorized
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [CURVE_MARKET_SEED, curve_market.market_id.as_ref()],
        bump = curve_market.bump,
        has_one = config,
        constraint = curve_market.vault == curve_vault.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_market: Box<Account<'info, CurveMarket>>,
    #[account(
        constraint = curve_vault.mint == curve_market.collateral_mint @ ExchangeError::WrongCollateralMint,
        constraint = curve_vault.owner == curve_market.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_vault: Box<Account<'info, TokenAccount>>,
}

#[derive(Accounts)]
pub struct ClaimCurvePosition<'info> {
    #[account(
        mut,
        seeds = [CURVE_MARKET_SEED, curve_market.market_id.as_ref()],
        bump = curve_market.bump,
        has_one = collateral_mint @ ExchangeError::WrongCollateralMint,
        constraint = curve_market.vault == curve_vault.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_market: Box<Account<'info, CurveMarket>>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = owner_collateral.owner == owner.key() @ ExchangeError::Unauthorized,
        constraint = owner_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub owner_collateral: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [
            CURVE_POSITION_SEED,
            curve_market.key().as_ref(),
            owner.key().as_ref(),
            &[position.bucket]
        ],
        bump = position.bump,
        constraint = position.market == curve_market.key() @ ExchangeError::WrongCurveMarket,
        has_one = owner @ ExchangeError::Unauthorized
    )]
    pub position: Box<Account<'info, CurvePosition>>,
    #[account(
        mut,
        constraint = curve_vault.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint,
        constraint = curve_vault.owner == curve_market.key() @ ExchangeError::WrongCurveVault
    )]
    pub curve_vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> ClaimCurvePosition<'info> {
    fn transfer_curve_payout(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.curve_vault.to_account_info(),
                to: self.owner_collateral.to_account_info(),
                authority: self.curve_market.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
#[instruction(order_id: [u8; 32])]
pub struct PlaceOrder<'info> {
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        has_one = collateral_mint @ ExchangeError::WrongCollateralMint,
        has_one = fee_recipient @ ExchangeError::WrongFeeRecipient
    )]
    pub config: Box<Account<'info, ExchangeConfig>>,
    #[account(
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = config,
        has_one = collateral_mint
    )]
    pub market: Box<Account<'info, ScarcityMarket>>,
    #[account(mut)]
    pub maker: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    pub outcome_mint: Box<Account<'info, Mint>>,
    pub fee_recipient: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = maker_source.owner == maker.key() @ ExchangeError::Unauthorized
    )]
    pub maker_source: Box<Account<'info, TokenAccount>>,
    pub escrow_mint: Box<Account<'info, Mint>>,
    #[account(
        init,
        payer = maker,
        space = 8 + LimitOrder::INIT_SPACE,
        seeds = [ORDER_SEED, market.key().as_ref(), maker.key().as_ref(), order_id.as_ref()],
        bump
    )]
    pub order: Box<Account<'info, LimitOrder>>,
    #[account(
        init,
        payer = maker,
        seeds = [ORDER_VAULT_SEED, order.key().as_ref()],
        bump,
        token::mint = escrow_mint,
        token::authority = order
    )]
    pub order_vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

impl<'info> PlaceOrder<'info> {
    fn transfer_to_order_vault(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.maker_source.to_account_info(),
                to: self.order_vault.to_account_info(),
                authority: self.maker.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct FillAsk<'info> {
    #[account(
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = collateral_mint
    )]
    pub market: Box<Account<'info, ScarcityMarket>>,
    #[account(
        mut,
        seeds = [
            ORDER_SEED,
            market.key().as_ref(),
            order.maker.as_ref(),
            order.order_id.as_ref()
        ],
        bump = order.bump,
        has_one = market,
        has_one = collateral_mint,
        has_one = outcome_mint,
        has_one = vault,
        has_one = fee_recipient @ ExchangeError::WrongFeeRecipient
    )]
    pub order: Box<Account<'info, LimitOrder>>,
    #[account(mut)]
    pub taker: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    pub outcome_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = maker_collateral.owner == order.maker @ ExchangeError::Unauthorized,
        constraint = maker_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub maker_collateral: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = taker_collateral.owner == taker.key() @ ExchangeError::Unauthorized,
        constraint = taker_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub taker_collateral: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = taker_outcome.owner == taker.key() @ ExchangeError::Unauthorized,
        constraint = taker_outcome.mint == outcome_mint.key() @ ExchangeError::WrongOutcomeMint
    )]
    pub taker_outcome: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = fee_recipient.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub fee_recipient: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> FillAsk<'info> {
    fn transfer_quote_to_maker(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.taker_collateral.to_account_info(),
                to: self.maker_collateral.to_account_info(),
                authority: self.taker.to_account_info(),
            },
        )
    }

    fn transfer_fee_from_taker(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.taker_collateral.to_account_info(),
                to: self.fee_recipient.to_account_info(),
                authority: self.taker.to_account_info(),
            },
        )
    }

    fn transfer_outcome_to_taker(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.vault.to_account_info(),
                to: self.taker_outcome.to_account_info(),
                authority: self.order.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct FillBid<'info> {
    #[account(
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
        has_one = collateral_mint
    )]
    pub market: Box<Account<'info, ScarcityMarket>>,
    #[account(
        mut,
        seeds = [
            ORDER_SEED,
            market.key().as_ref(),
            order.maker.as_ref(),
            order.order_id.as_ref()
        ],
        bump = order.bump,
        has_one = market,
        has_one = collateral_mint,
        has_one = outcome_mint,
        has_one = vault,
        has_one = fee_recipient @ ExchangeError::WrongFeeRecipient
    )]
    pub order: Box<Account<'info, LimitOrder>>,
    #[account(mut)]
    pub taker: Signer<'info>,
    pub collateral_mint: Box<Account<'info, Mint>>,
    pub outcome_mint: Box<Account<'info, Mint>>,
    #[account(
        mut,
        constraint = maker_outcome.owner == order.maker @ ExchangeError::Unauthorized,
        constraint = maker_outcome.mint == outcome_mint.key() @ ExchangeError::WrongOutcomeMint
    )]
    pub maker_outcome: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = taker_collateral.owner == taker.key() @ ExchangeError::Unauthorized,
        constraint = taker_collateral.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub taker_collateral: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = taker_outcome.owner == taker.key() @ ExchangeError::Unauthorized,
        constraint = taker_outcome.mint == outcome_mint.key() @ ExchangeError::WrongOutcomeMint
    )]
    pub taker_outcome: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = fee_recipient.mint == collateral_mint.key() @ ExchangeError::WrongCollateralMint
    )]
    pub fee_recipient: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> FillBid<'info> {
    fn transfer_outcome_to_maker(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.taker_outcome.to_account_info(),
                to: self.maker_outcome.to_account_info(),
                authority: self.taker.to_account_info(),
            },
        )
    }

    fn transfer_quote_to_taker(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.vault.to_account_info(),
                to: self.taker_collateral.to_account_info(),
                authority: self.order.to_account_info(),
            },
        )
    }

    fn transfer_fee_from_vault(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.vault.to_account_info(),
                to: self.fee_recipient.to_account_info(),
                authority: self.order.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct CancelOrder<'info> {
    pub market: Box<Account<'info, ScarcityMarket>>,
    #[account(
        mut,
        close = maker,
        seeds = [
            ORDER_SEED,
            market.key().as_ref(),
            order.maker.as_ref(),
            order.order_id.as_ref()
        ],
        bump = order.bump,
        has_one = market,
        has_one = maker @ ExchangeError::Unauthorized,
        has_one = escrow_mint,
        has_one = vault
    )]
    pub order: Box<Account<'info, LimitOrder>>,
    #[account(mut)]
    pub maker: Signer<'info>,
    pub escrow_mint: Box<Account<'info, Mint>>,
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        constraint = maker_refund.owner == maker.key() @ ExchangeError::Unauthorized,
        constraint = maker_refund.mint == escrow_mint.key() @ ExchangeError::WrongEscrowMint
    )]
    pub maker_refund: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

impl<'info> CancelOrder<'info> {
    fn transfer_refund_to_maker(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: self.vault.to_account_info(),
                to: self.maker_refund.to_account_info(),
                authority: self.order.to_account_info(),
            },
        )
    }

    fn close_order_vault(&self) -> CpiContext<'_, '_, '_, 'info, CloseAccount<'info>> {
        CpiContext::new(
            self.token_program.to_account_info(),
            CloseAccount {
                account: self.vault.to_account_info(),
                destination: self.maker.to_account_info(),
                authority: self.order.to_account_info(),
            },
        )
    }
}

#[account]
#[derive(InitSpace)]
pub struct ExchangeConfig {
    pub version: u8,
    pub bump: u8,
    pub paused: bool,
    pub admin: Pubkey,
    pub resolver: Pubkey,
    pub collateral_mint: Pubkey,
    pub fee_recipient: Pubkey,
    pub trading_fee_bps: u16,
}

#[account]
#[derive(InitSpace)]
pub struct ScarcityMarket {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub status: MarketStatus,
    pub config: Pubkey,
    pub creator: Pubkey,
    pub resolver: Pubkey,
    pub collateral_mint: Pubkey,
    pub yes_mint: Pubkey,
    pub no_mint: Pubkey,
    pub vault: Pubkey,
    pub market_id: [u8; 32],
    pub question_hash: [u8; 32],
    pub rules_hash: [u8; 32],
    pub resolution_report_hash: [u8; 32],
    pub opens_at: i64,
    pub closes_at: i64,
    pub resolve_after: i64,
    pub resolved_at: i64,
    pub open_interest: u64,
    pub total_redeemed: u64,
}

#[account]
#[derive(InitSpace)]
pub struct CurveMarket {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub status: CurveMarketStatus,
    pub kernel_version: u8,
    pub jackpot_leverage_cap: u8,
    pub config: Pubkey,
    pub creator: Pubkey,
    pub resolver: Pubkey,
    pub collateral_mint: Pubkey,
    pub vault: Pubkey,
    pub fee_recipient: Pubkey,
    pub market_id: [u8; 32],
    pub metric_hash: [u8; 32],
    pub rules_hash: [u8; 32],
    pub resolution_report_hash: [u8; 32],
    pub opens_at: i64,
    pub closes_at: i64,
    pub resolve_after: i64,
    pub resolved_at: i64,
    pub normalized_outcome: i32,
    pub bucket_count: u8,
    pub winning_bucket: u8,
    pub jackpot_bps: u16,
    pub fee_bps: u16,
    pub total_staked: u64,
    pub protocol_fee: u64,
    pub payout_pool: u64,
    pub jackpot_pool: u64,
    pub curve_pool: u64,
    pub exact_stake: u64,
    pub weighted_stake: u64,
    pub total_claimed: u64,
    pub bucket_stakes: [u64; MAX_CURVE_BUCKETS as usize],
}

#[account]
#[derive(InitSpace)]
pub struct CurvePosition {
    pub version: u8,
    pub bump: u8,
    pub market: Pubkey,
    pub owner: Pubkey,
    pub bucket: u8,
    pub claimed: bool,
    pub stake: u64,
    pub payout: u64,
}

#[account]
#[derive(InitSpace)]
pub struct LimitOrder {
    pub version: u8,
    pub bump: u8,
    pub vault_bump: u8,
    pub side: OrderSide,
    pub market: Pubkey,
    pub maker: Pubkey,
    pub collateral_mint: Pubkey,
    pub outcome_mint: Pubkey,
    pub escrow_mint: Pubkey,
    pub vault: Pubkey,
    pub fee_recipient: Pubkey,
    pub order_id: [u8; 32],
    pub price_micro_usdc: u64,
    pub original_quantity: u64,
    pub remaining_quantity: u64,
    pub quote_filled: u64,
    pub fee_paid: u64,
    pub fee_bps: u16,
    pub expires_at: i64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum MarketStatus {
    Unresolved,
    ResolvedYes,
    ResolvedNo,
    Invalid,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum CurveMarketStatus {
    Unresolved,
    Resolved,
    Invalid,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionOutcome {
    Yes,
    No,
    Invalid,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum OrderSide {
    Bid,
    Ask,
}

fn redemption_payout(
    status: MarketStatus,
    claim_mint: Pubkey,
    market: &ScarcityMarket,
    amount: u64,
) -> Result<u64> {
    match status {
        MarketStatus::Unresolved => err!(ExchangeError::MarketNotResolved),
        MarketStatus::ResolvedYes => {
            require_keys_eq!(claim_mint, market.yes_mint, ExchangeError::LosingOutcome);
            Ok(amount)
        }
        MarketStatus::ResolvedNo => {
            require_keys_eq!(claim_mint, market.no_mint, ExchangeError::LosingOutcome);
            Ok(amount)
        }
        MarketStatus::Invalid => {
            require!(
                claim_mint == market.yes_mint || claim_mint == market.no_mint,
                ExchangeError::WrongOutcomeMint
            );
            Ok(amount / 2)
        }
    }
}

fn quote_for_quantity(quantity: u64, price_micro_usdc: u64) -> Result<u64> {
    ceil_mul_div(
        u128::from(quantity),
        u128::from(price_micro_usdc),
        PRICE_SCALE,
    )
}

fn fee_for_quote(quote: u64, fee_bps: u16) -> Result<u64> {
    if fee_bps == 0 || quote == 0 {
        return Ok(0);
    }
    ceil_mul_div(u128::from(quote), u128::from(fee_bps), BPS_SCALE)
}

fn ceil_mul_div(left: u128, right: u128, denominator: u128) -> Result<u64> {
    require!(denominator > 0, ExchangeError::ArithmeticOverflow);
    let product = left
        .checked_mul(right)
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?;
    let rounded = product
        .checked_add(denominator - 1)
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?
        / denominator;
    u64::try_from(rounded).map_err(|_| error!(ExchangeError::ArithmeticOverflow))
}

fn fill_amounts(order: &LimitOrder, quantity: u64) -> Result<(u64, u64, u64, u64)> {
    let filled_before = checked_sub(order.original_quantity, order.remaining_quantity)?;
    let filled_after = checked_add(filled_before, quantity)?;
    let quote_after = quote_for_quantity(filled_after, order.price_micro_usdc)?;
    let fee_after = fee_for_quote(quote_after, order.fee_bps)?;
    let quote_delta = checked_sub(quote_after, order.quote_filled)?;
    let fee_delta = checked_sub(fee_after, order.fee_paid)?;
    Ok((quote_delta, fee_delta, quote_after, fee_after))
}

fn validate_fill(
    market: &ScarcityMarket,
    order: &LimitOrder,
    expected_side: OrderSide,
    quantity: u64,
) -> Result<()> {
    require!(quantity > 0, ExchangeError::InvalidAmount);
    require!(order.side == expected_side, ExchangeError::WrongOrderSide);
    require!(
        quantity <= order.remaining_quantity,
        ExchangeError::AmountExceedsOrder
    );
    require!(
        market.status == MarketStatus::Unresolved,
        ExchangeError::MarketResolved
    );
    let now = Clock::get()?.unix_timestamp;
    require!(now < order.expires_at, ExchangeError::OrderExpired);
    require!(now < market.closes_at, ExchangeError::MarketClosed);
    Ok(())
}

fn validate_curve_parameters(bucket_count: u8, jackpot_bps: u16) -> Result<()> {
    require!(
        (MIN_CURVE_BUCKETS..=MAX_CURVE_BUCKETS).contains(&bucket_count) && bucket_count % 2 == 1,
        ExchangeError::InvalidCurveBucketCount
    );
    require!(
        jackpot_bps <= MAX_CURVE_JACKPOT_BPS,
        ExchangeError::CurveJackpotTooHigh
    );
    Ok(())
}

fn curve_jackpot_leverage_cap(bucket_count: u8) -> Result<u8> {
    validate_curve_parameters(bucket_count, 0)?;
    Ok(MAX_CURVE_JACKPOT_LEVERAGE.min(bucket_count - 1))
}

fn curve_recovery_at(resolve_after: i64) -> Result<i64> {
    resolve_after
        .checked_add(CURVE_RESOLVER_RECOVERY_DELAY_SECONDS)
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))
}

fn validate_curve_staking_window(market: &CurveMarket) -> Result<()> {
    require!(
        market.status == CurveMarketStatus::Unresolved,
        ExchangeError::MarketResolved
    );
    let now = Clock::get()?.unix_timestamp;
    require!(now >= market.opens_at, ExchangeError::MarketNotOpen);
    require!(now < market.closes_at, ExchangeError::MarketClosed);
    Ok(())
}

fn max_curve_total_stake(bucket_count: u8) -> Result<u64> {
    require!(bucket_count > 0, ExchangeError::InvalidCurveBucketCount);
    Ok(u64::MAX / u64::from(bucket_count))
}

fn curve_bucket_for_value(normalized_value: i32, bucket_count: u8) -> Result<u8> {
    validate_curve_parameters(bucket_count, 0)?;
    require!(
        (-CURVE_VALUE_SCALE..=CURVE_VALUE_SCALE).contains(&normalized_value),
        ExchangeError::InvalidCurveValue
    );

    let shifted = i64::from(normalized_value)
        .checked_add(i64::from(CURVE_VALUE_SCALE))
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?;
    let intervals = u64::from(bucket_count - 1);
    let numerator =
        u128::from(u64::try_from(shifted).map_err(|_| error!(ExchangeError::ArithmeticOverflow))?)
            .checked_mul(u128::from(intervals))
            .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?;
    let span = u128::from(
        u64::try_from(i64::from(CURVE_VALUE_SCALE) * 2)
            .map_err(|_| error!(ExchangeError::ArithmeticOverflow))?,
    );
    let rounded = numerator
        .checked_add(span / 2)
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?
        / span;
    u8::try_from(rounded).map_err(|_| error!(ExchangeError::ArithmeticOverflow))
}

fn curve_weight(bucket: u8, winning_bucket: u8, bucket_count: u8) -> Result<u64> {
    require!(
        bucket < bucket_count && winning_bucket < bucket_count,
        ExchangeError::InvalidCurveBucket
    );
    let distance = bucket.abs_diff(winning_bucket);
    Ok(u64::from(bucket_count - distance))
}

fn curve_weighted_total(
    bucket_stakes: &[u64; MAX_CURVE_BUCKETS as usize],
    bucket_count: u8,
    winning_bucket: u8,
) -> Result<u64> {
    validate_curve_parameters(bucket_count, 0)?;
    require!(
        winning_bucket < bucket_count,
        ExchangeError::InvalidCurveBucket
    );

    let mut total = 0u64;
    for bucket in 0..bucket_count {
        let weighted = bucket_stakes[usize::from(bucket)]
            .checked_mul(curve_weight(bucket, winning_bucket, bucket_count)?)
            .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?;
        total = checked_add(total, weighted)?;
    }
    Ok(total)
}

fn curve_recorded_stake_total(
    bucket_stakes: &[u64; MAX_CURVE_BUCKETS as usize],
    bucket_count: u8,
) -> Result<u64> {
    validate_curve_parameters(bucket_count, 0)?;
    let mut total = 0u64;
    for (index, stake) in bucket_stakes.iter().enumerate() {
        if index >= usize::from(bucket_count) {
            require!(*stake == 0, ExchangeError::InvalidCurveAccounting);
        }
        total = checked_add(total, *stake)?;
    }
    Ok(total)
}

fn floor_mul_div_u64(left: u64, right: u64, denominator: u64) -> Result<u64> {
    require!(denominator > 0, ExchangeError::ArithmeticOverflow);
    let value = u128::from(left)
        .checked_mul(u128::from(right))
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?
        / u128::from(denominator);
    u64::try_from(value).map_err(|_| error!(ExchangeError::ArithmeticOverflow))
}

fn curve_position_payout(market: &CurveMarket, position: &CurvePosition) -> Result<u64> {
    require!(
        position.market != Pubkey::default(),
        ExchangeError::WrongCurveMarket
    );
    require!(
        position.bucket < market.bucket_count,
        ExchangeError::InvalidCurveBucket
    );

    match market.status {
        CurveMarketStatus::Unresolved => err!(ExchangeError::MarketNotResolved),
        CurveMarketStatus::Invalid => Ok(position.stake),
        CurveMarketStatus::Resolved => {
            if position.stake == 0 {
                return Ok(0);
            }
            require!(
                market.winning_bucket < market.bucket_count,
                ExchangeError::InvalidCurveBucket
            );
            require!(
                market.weighted_stake > 0,
                ExchangeError::InvalidCurveDenominator
            );

            let weight = curve_weight(position.bucket, market.winning_bucket, market.bucket_count)?;
            let weighted_position = position
                .stake
                .checked_mul(weight)
                .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))?;
            let curve_payout =
                floor_mul_div_u64(weighted_position, market.curve_pool, market.weighted_stake)?;
            let jackpot_payout =
                if position.bucket == market.winning_bucket && market.jackpot_pool > 0 {
                    require!(
                        market.exact_stake > 0,
                        ExchangeError::InvalidCurveDenominator
                    );
                    floor_mul_div_u64(position.stake, market.jackpot_pool, market.exact_stake)?
                } else {
                    0
                };
            checked_add(curve_payout, jackpot_payout)
        }
    }
}

fn checked_add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))
}

fn checked_sub(left: u64, right: u64) -> Result<u64> {
    left.checked_sub(right)
        .ok_or_else(|| error!(ExchangeError::ArithmeticOverflow))
}

#[event]
pub struct ConfigInitialized {
    pub admin: Pubkey,
    pub resolver: Pubkey,
    pub collateral_mint: Pubkey,
    pub fee_recipient: Pubkey,
    pub trading_fee_bps: u16,
}

#[event]
pub struct PauseChanged {
    pub paused: bool,
}

#[event]
pub struct ResolverChanged {
    pub previous_resolver: Pubkey,
    pub resolver: Pubkey,
}

#[event]
pub struct MarketCreated {
    pub market: Pubkey,
    pub market_id: [u8; 32],
    pub question_hash: [u8; 32],
    pub rules_hash: [u8; 32],
    pub resolver: Pubkey,
    pub yes_mint: Pubkey,
    pub no_mint: Pubkey,
    pub opens_at: i64,
    pub closes_at: i64,
    pub resolve_after: i64,
}

#[event]
pub struct CompleteSetMinted {
    pub market: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub open_interest: u64,
}

#[event]
pub struct CompleteSetMerged {
    pub market: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub open_interest: u64,
}

#[event]
pub struct MarketResolved {
    pub market: Pubkey,
    pub outcome: ResolutionOutcome,
    pub resolution_report_hash: [u8; 32],
    pub open_interest: u64,
    pub resolved_at: i64,
}

#[event]
pub struct PositionRedeemed {
    pub market: Pubkey,
    pub owner: Pubkey,
    pub claim_mint: Pubkey,
    pub burned_amount: u64,
    pub payout: u64,
    pub total_redeemed: u64,
}

#[event]
pub struct OrderPlaced {
    pub order: Pubkey,
    pub market: Pubkey,
    pub maker: Pubkey,
    pub side: OrderSide,
    pub outcome_mint: Pubkey,
    pub price_micro_usdc: u64,
    pub quantity: u64,
    pub escrow_amount: u64,
    pub fee_bps: u16,
    pub expires_at: i64,
}

#[event]
pub struct OrderFilled {
    pub order: Pubkey,
    pub market: Pubkey,
    pub maker: Pubkey,
    pub taker: Pubkey,
    pub side: OrderSide,
    pub quantity: u64,
    pub quote_amount: u64,
    pub fee_amount: u64,
    pub remaining_quantity: u64,
}

#[event]
pub struct OrderCancelled {
    pub order: Pubkey,
    pub market: Pubkey,
    pub maker: Pubkey,
    pub refund_amount: u64,
    pub unfilled_quantity: u64,
}

#[event]
pub struct CurveMarketCreated {
    pub market: Pubkey,
    pub market_id: [u8; 32],
    pub metric_hash: [u8; 32],
    pub rules_hash: [u8; 32],
    pub resolver: Pubkey,
    pub collateral_mint: Pubkey,
    pub bucket_count: u8,
    pub jackpot_bps: u16,
    pub fee_bps: u16,
    pub kernel_version: u8,
    pub jackpot_leverage_cap: u8,
    pub opens_at: i64,
    pub closes_at: i64,
    pub resolve_after: i64,
}

#[event]
pub struct CurvePositionOpened {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub bucket: u8,
    pub amount: u64,
    pub position_stake: u64,
    pub total_staked: u64,
}

#[event]
pub struct CurveStakeAdded {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub bucket: u8,
    pub amount: u64,
    pub position_stake: u64,
    pub total_staked: u64,
}

#[event]
pub struct CurveStakeWithdrawn {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub bucket: u8,
    pub amount: u64,
    pub position_stake: u64,
    pub total_staked: u64,
}

#[event]
pub struct MarketClosed {
    pub market: Pubkey,
    pub market_id: [u8; 32],
    pub open_interest: u64,
    pub total_redeemed: u64,
}

#[event]
pub struct CurveMarketResolved {
    pub market: Pubkey,
    pub normalized_outcome: i32,
    pub winning_bucket: u8,
    pub resolution_report_hash: [u8; 32],
    pub total_staked: u64,
    pub protocol_fee: u64,
    pub payout_pool: u64,
    pub jackpot_pool: u64,
    pub curve_pool: u64,
    pub exact_stake: u64,
    pub weighted_stake: u64,
    pub resolved_at: i64,
}

#[event]
pub struct CurveMarketInvalidated {
    pub market: Pubkey,
    pub resolution_report_hash: [u8; 32],
    pub total_staked: u64,
    pub resolved_at: i64,
}

#[event]
pub struct CurveMarketRecovered {
    pub market: Pubkey,
    pub admin: Pubkey,
    pub resolution_report_hash: [u8; 32],
    pub total_staked: u64,
    pub recovered_at: i64,
}

#[event]
pub struct CurvePositionClaimed {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub bucket: u8,
    pub stake: u64,
    pub payout: u64,
    pub total_claimed: u64,
}

#[error_code]
pub enum ExchangeError {
    #[msg("Only the configured authority may perform this action.")]
    Unauthorized,
    #[msg("The resolver public key is invalid.")]
    InvalidResolver,
    #[msg("New issuance is paused.")]
    IssuancePaused,
    #[msg("The market identifier is invalid.")]
    InvalidMarketId,
    #[msg("A required content commitment is invalid.")]
    InvalidCommitment,
    #[msg("The market schedule is invalid.")]
    InvalidSchedule,
    #[msg("The amount must be greater than zero.")]
    InvalidAmount,
    #[msg("The market is not open yet.")]
    MarketNotOpen,
    #[msg("The market is closed to new issuance.")]
    MarketClosed,
    #[msg("The market has already been resolved.")]
    MarketResolved,
    #[msg("The market has not been resolved.")]
    MarketNotResolved,
    #[msg("The market cannot be resolved yet.")]
    ResolutionTooEarly,
    #[msg("The supplied collateral mint is not accepted.")]
    WrongCollateralMint,
    #[msg("The collateral mint must use six decimal places.")]
    InvalidCollateralDecimals,
    #[msg("The supplied outcome mint does not belong to this market.")]
    WrongOutcomeMint,
    #[msg("The supplied outcome did not win this market.")]
    LosingOutcome,
    #[msg("The amount exceeds the market's remaining collateral liability.")]
    AmountExceedsLiability,
    #[msg("The amount is too small to produce a payout.")]
    AmountTooSmall,
    #[msg("Arithmetic overflow or underflow.")]
    ArithmeticOverflow,
    #[msg("The configured trading fee exceeds the protocol maximum.")]
    FeeTooHigh,
    #[msg("The order identifier is invalid.")]
    InvalidOrderId,
    #[msg("The outcome price must be greater than zero and no more than one USDC.")]
    InvalidPrice,
    #[msg("The order has expired.")]
    OrderExpired,
    #[msg("The escrow mint does not match the order side.")]
    WrongEscrowMint,
    #[msg("The fee recipient does not match the snapshotted order configuration.")]
    WrongFeeRecipient,
    #[msg("The order side does not match this fill instruction.")]
    WrongOrderSide,
    #[msg("The requested fill exceeds the order's remaining quantity.")]
    AmountExceedsOrder,
    #[msg("A curve market must use an odd bucket count between three and forty-one.")]
    InvalidCurveBucketCount,
    #[msg("The curve jackpot share exceeds the protocol maximum.")]
    CurveJackpotTooHigh,
    #[msg("The normalized curve value must be between negative one and positive one.")]
    InvalidCurveValue,
    #[msg("The curve bucket is outside this market's range.")]
    InvalidCurveBucket,
    #[msg("The curve stake cap has been reached.")]
    CurveStakeLimitExceeded,
    #[msg("The requested amount exceeds this curve position's stake.")]
    AmountExceedsPosition,
    #[msg("This curve position has already been claimed.")]
    PositionAlreadyClaimed,
    #[msg("The curve vault does not belong to this market.")]
    WrongCurveVault,
    #[msg("The curve position does not belong to this market.")]
    WrongCurveMarket,
    #[msg("The curve vault does not contain the market's recorded collateral.")]
    InsufficientCurveCollateral,
    #[msg("The curve payout denominator is invalid.")]
    InvalidCurveDenominator,
    #[msg("The curve market's aggregate stake accounting is inconsistent.")]
    InvalidCurveAccounting,
    #[msg("The resolver recovery delay has not elapsed.")]
    CurveRecoveryTooEarly,
    #[msg("Two accounts that must be distinct resolve to the same address.")]
    DuplicateAccount,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn market(status: MarketStatus) -> ScarcityMarket {
        ScarcityMarket {
            version: 1,
            bump: 1,
            vault_bump: 2,
            status,
            config: Pubkey::new_unique(),
            creator: Pubkey::new_unique(),
            resolver: Pubkey::new_unique(),
            collateral_mint: Pubkey::new_unique(),
            yes_mint: Pubkey::new_unique(),
            no_mint: Pubkey::new_unique(),
            vault: Pubkey::new_unique(),
            market_id: [1; 32],
            question_hash: [2; 32],
            rules_hash: [3; 32],
            resolution_report_hash: [4; 32],
            opens_at: 1,
            closes_at: 2,
            resolve_after: 3,
            resolved_at: 4,
            open_interest: 1_000_000,
            total_redeemed: 0,
        }
    }

    fn order(price_micro_usdc: u64, quantity: u64, fee_bps: u16) -> LimitOrder {
        LimitOrder {
            version: 1,
            bump: 1,
            vault_bump: 2,
            side: OrderSide::Ask,
            market: Pubkey::new_unique(),
            maker: Pubkey::new_unique(),
            collateral_mint: Pubkey::new_unique(),
            outcome_mint: Pubkey::new_unique(),
            escrow_mint: Pubkey::new_unique(),
            vault: Pubkey::new_unique(),
            fee_recipient: Pubkey::new_unique(),
            order_id: [9; 32],
            price_micro_usdc,
            original_quantity: quantity,
            remaining_quantity: quantity,
            quote_filled: 0,
            fee_paid: 0,
            fee_bps,
            expires_at: i64::MAX,
        }
    }

    fn curve_market(status: CurveMarketStatus, bucket_count: u8) -> CurveMarket {
        CurveMarket {
            version: 1,
            bump: 1,
            vault_bump: 2,
            status,
            kernel_version: CURVE_KERNEL_VERSION,
            jackpot_leverage_cap: curve_jackpot_leverage_cap(bucket_count).unwrap(),
            config: Pubkey::new_unique(),
            creator: Pubkey::new_unique(),
            resolver: Pubkey::new_unique(),
            collateral_mint: Pubkey::new_unique(),
            vault: Pubkey::new_unique(),
            fee_recipient: Pubkey::new_unique(),
            market_id: [11; 32],
            metric_hash: [12; 32],
            rules_hash: [13; 32],
            resolution_report_hash: [14; 32],
            opens_at: 1,
            closes_at: 2,
            resolve_after: 3,
            resolved_at: 4,
            normalized_outcome: 0,
            bucket_count,
            winning_bucket: u8::MAX,
            jackpot_bps: 2_000,
            fee_bps: 25,
            total_staked: 0,
            protocol_fee: 0,
            payout_pool: 0,
            jackpot_pool: 0,
            curve_pool: 0,
            exact_stake: 0,
            weighted_stake: 0,
            total_claimed: 0,
            bucket_stakes: [0; MAX_CURVE_BUCKETS as usize],
        }
    }

    fn curve_position(market: Pubkey, bucket: u8, stake: u64) -> CurvePosition {
        CurvePosition {
            version: 1,
            bump: 1,
            market,
            owner: Pubkey::new_unique(),
            bucket,
            claimed: false,
            stake,
            payout: 0,
        }
    }

    fn settle_curve_fixture(market: &mut CurveMarket, normalized_outcome: i32) {
        market.status = CurveMarketStatus::Resolved;
        market.normalized_outcome = normalized_outcome;
        market.winning_bucket =
            curve_bucket_for_value(normalized_outcome, market.bucket_count).unwrap();
        market.exact_stake = market.bucket_stakes[usize::from(market.winning_bucket)];
        market.weighted_stake = curve_weighted_total(
            &market.bucket_stakes,
            market.bucket_count,
            market.winning_bucket,
        )
        .unwrap();
        market.protocol_fee = floor_mul_div_u64(
            market.total_staked,
            u64::from(market.fee_bps),
            BPS_SCALE as u64,
        )
        .unwrap();
        market.payout_pool = market.total_staked - market.protocol_fee;
        let target_jackpot = floor_mul_div_u64(
            market.payout_pool,
            u64::from(market.jackpot_bps),
            BPS_SCALE as u64,
        )
        .unwrap();
        let jackpot_cap = market
            .exact_stake
            .checked_mul(u64::from(market.jackpot_leverage_cap))
            .unwrap();
        market.jackpot_pool = target_jackpot.min(jackpot_cap);
        market.curve_pool = market.payout_pool - market.jackpot_pool;
    }

    #[test]
    fn yes_market_pays_yes_one_for_one() {
        let market = market(MarketStatus::ResolvedYes);
        assert_eq!(
            redemption_payout(market.status, market.yes_mint, &market, 250_000).unwrap(),
            250_000
        );
        assert!(redemption_payout(market.status, market.no_mint, &market, 250_000).is_err());
    }

    #[test]
    fn no_market_pays_no_one_for_one() {
        let market = market(MarketStatus::ResolvedNo);
        assert_eq!(
            redemption_payout(market.status, market.no_mint, &market, 250_000).unwrap(),
            250_000
        );
        assert!(redemption_payout(market.status, market.yes_mint, &market, 250_000).is_err());
    }

    #[test]
    fn invalid_market_splits_collateral_evenly() {
        let market = market(MarketStatus::Invalid);
        assert_eq!(
            redemption_payout(market.status, market.yes_mint, &market, 500_000).unwrap(),
            250_000
        );
        assert_eq!(
            redemption_payout(market.status, market.no_mint, &market, 500_000).unwrap(),
            250_000
        );
    }

    #[test]
    fn unresolved_market_cannot_redeem() {
        let market = market(MarketStatus::Unresolved);
        assert!(redemption_payout(market.status, market.yes_mint, &market, 1).is_err());
    }

    #[test]
    fn checked_math_fails_closed() {
        assert!(checked_add(u64::MAX, 1).is_err());
        assert!(checked_sub(0, 1).is_err());
    }

    #[test]
    fn order_quote_uses_exact_six_decimal_units() {
        assert_eq!(quote_for_quantity(1_000_000, 640_000).unwrap(), 640_000);
        assert_eq!(quote_for_quantity(250_000, 640_000).unwrap(), 160_000);
        assert_eq!(fee_for_quote(640_000, 50).unwrap(), 3_200);
    }

    #[test]
    fn cumulative_rounding_never_overcharges_partial_fills() {
        let mut order = order(333_333, 1_000_000, 50);
        let first = fill_amounts(&order, 333_333).unwrap();
        order.remaining_quantity -= 333_333;
        order.quote_filled = first.2;
        order.fee_paid = first.3;
        let second = fill_amounts(&order, 666_667).unwrap();

        let full_quote = quote_for_quantity(1_000_000, 333_333).unwrap();
        let full_fee = fee_for_quote(full_quote, 50).unwrap();
        assert_eq!(first.0 + second.0, full_quote);
        assert_eq!(first.1 + second.1, full_fee);
    }

    #[test]
    fn curve_parameters_are_bounded_and_odd() {
        assert!(validate_curve_parameters(3, 0).is_ok());
        assert!(validate_curve_parameters(41, 5_000).is_ok());
        assert!(validate_curve_parameters(2, 0).is_err());
        assert!(validate_curve_parameters(4, 0).is_err());
        assert!(validate_curve_parameters(43, 0).is_err());
        assert!(validate_curve_parameters(41, 5_001).is_err());
    }

    #[test]
    fn curve_account_sizes_are_stable_for_generated_clients() {
        assert_eq!(8 + CurveMarket::INIT_SPACE, 768);
        assert_eq!(8 + CurvePosition::INIT_SPACE, 92);
    }

    #[test]
    fn curve_jackpot_leverage_is_bounded_below_bucket_count() {
        assert_eq!(curve_jackpot_leverage_cap(3).unwrap(), 2);
        assert_eq!(curve_jackpot_leverage_cap(9).unwrap(), 8);
        assert_eq!(curve_jackpot_leverage_cap(41).unwrap(), 10);
    }

    #[test]
    fn normalized_values_map_to_canonical_buckets() {
        assert_eq!(curve_bucket_for_value(-1_000_000, 41).unwrap(), 0);
        assert_eq!(curve_bucket_for_value(0, 41).unwrap(), 20);
        assert_eq!(curve_bucket_for_value(1_000_000, 41).unwrap(), 40);
        assert_eq!(curve_bucket_for_value(24_999, 41).unwrap(), 20);
        assert_eq!(curve_bucket_for_value(25_000, 41).unwrap(), 21);
        assert_eq!(curve_bucket_for_value(-25_000, 41).unwrap(), 20);
        assert!(curve_bucket_for_value(-1_000_001, 41).is_err());
        assert!(curve_bucket_for_value(1_000_001, 41).is_err());
    }

    #[test]
    fn curve_kernel_has_full_support_and_rewards_accuracy() {
        assert_eq!(curve_weight(20, 20, 41).unwrap(), 41);
        assert_eq!(curve_weight(19, 20, 41).unwrap(), 40);
        assert_eq!(curve_weight(0, 20, 41).unwrap(), 21);
        assert_eq!(curve_weight(0, 40, 41).unwrap(), 1);
        assert!(curve_weight(41, 20, 41).is_err());
    }

    #[test]
    fn curve_bucket_ledger_reproduces_recorded_stake() {
        let mut stakes = [0u64; MAX_CURVE_BUCKETS as usize];
        stakes[0] = 10;
        stakes[20] = 20;
        stakes[40] = 30;
        assert_eq!(curve_recorded_stake_total(&stakes, 41).unwrap(), 60);

        let mut inactive_stake = [0u64; MAX_CURVE_BUCKETS as usize];
        inactive_stake[5] = 1;
        assert!(curve_recorded_stake_total(&inactive_stake, 5).is_err());
    }

    #[test]
    fn curve_jackpot_and_accuracy_pool_are_solvent() {
        let mut market = curve_market(CurveMarketStatus::Unresolved, 41);
        market.bucket_stakes[20] = 1_000_000;
        market.bucket_stakes[19] = 1_000_000;
        market.bucket_stakes[0] = 1_000_000;
        market.total_staked = 3_000_000;
        settle_curve_fixture(&mut market, 0);

        let market_key = Pubkey::new_unique();
        let exact = curve_position(market_key, 20, 1_000_000);
        let near = curve_position(market_key, 19, 1_000_000);
        let far = curve_position(market_key, 0, 1_000_000);
        let exact_payout = curve_position_payout(&market, &exact).unwrap();
        let near_payout = curve_position_payout(&market, &near).unwrap();
        let far_payout = curve_position_payout(&market, &far).unwrap();

        assert_eq!(market.protocol_fee, 7_500);
        assert_eq!(market.payout_pool, 2_992_500);
        assert_eq!(market.jackpot_pool, 598_500);
        assert!(exact_payout > near_payout);
        assert!(near_payout > far_payout);
        assert!(exact_payout + near_payout + far_payout <= market.payout_pool);
        assert!(
            exact_payout + near_payout + far_payout + market.protocol_fee <= market.total_staked
        );
    }

    #[test]
    fn missing_exact_bucket_rolls_jackpot_into_accuracy_pool() {
        let mut market = curve_market(CurveMarketStatus::Unresolved, 41);
        market.bucket_stakes[19] = 2_000_000;
        market.bucket_stakes[0] = 1_000_000;
        market.total_staked = 3_000_000;
        settle_curve_fixture(&mut market, 0);

        assert_eq!(market.exact_stake, 0);
        assert_eq!(market.jackpot_pool, 0);
        assert_eq!(market.curve_pool, market.payout_pool);

        let market_key = Pubkey::new_unique();
        let near = curve_position(market_key, 19, 2_000_000);
        let far = curve_position(market_key, 0, 1_000_000);
        let claimed = curve_position_payout(&market, &near).unwrap()
            + curve_position_payout(&market, &far).unwrap();
        assert!(claimed <= market.payout_pool);
    }

    #[test]
    fn dust_in_every_bucket_cannot_capture_a_fixed_jackpot() {
        let mut market = curve_market(CurveMarketStatus::Unresolved, 41);
        market.jackpot_bps = 5_000;
        for bucket in 0..41usize {
            market.bucket_stakes[bucket] = 1;
            market.total_staked += 1;
        }
        market.bucket_stakes[0] += 100_000_000;
        market.total_staked += 100_000_000;

        settle_curve_fixture(&mut market, 0);

        let target_jackpot = floor_mul_div_u64(
            market.payout_pool,
            u64::from(market.jackpot_bps),
            BPS_SCALE as u64,
        )
        .unwrap();
        assert!(target_jackpot > 10);
        assert_eq!(market.exact_stake, 1);
        assert_eq!(market.jackpot_pool, 10);
        assert_eq!(market.curve_pool, market.payout_pool - 10);
    }

    #[test]
    fn invalid_curve_market_refunds_stake_without_fee() {
        let mut market = curve_market(CurveMarketStatus::Invalid, 41);
        market.total_staked = 750_000;
        market.payout_pool = market.total_staked;
        market.curve_pool = market.total_staked;
        let position = curve_position(Pubkey::new_unique(), 7, 750_000);

        assert_eq!(curve_position_payout(&market, &position).unwrap(), 750_000);
        assert_eq!(market.protocol_fee, 0);
    }

    #[test]
    fn curve_payout_floors_remain_solvent_across_all_buckets() {
        for winning_bucket in 0..9u8 {
            let mut market = curve_market(CurveMarketStatus::Resolved, 9);
            for bucket in 0..9u8 {
                let stake = u64::from(bucket + 1) * 137_111;
                market.bucket_stakes[usize::from(bucket)] = stake;
                market.total_staked += stake;
            }
            market.winning_bucket = winning_bucket;
            market.normalized_outcome = 0;
            market.exact_stake = market.bucket_stakes[usize::from(winning_bucket)];
            market.weighted_stake =
                curve_weighted_total(&market.bucket_stakes, market.bucket_count, winning_bucket)
                    .unwrap();
            market.protocol_fee = floor_mul_div_u64(
                market.total_staked,
                u64::from(market.fee_bps),
                BPS_SCALE as u64,
            )
            .unwrap();
            market.payout_pool = market.total_staked - market.protocol_fee;
            let target_jackpot = floor_mul_div_u64(
                market.payout_pool,
                u64::from(market.jackpot_bps),
                BPS_SCALE as u64,
            )
            .unwrap();
            market.jackpot_pool = target_jackpot.min(
                market
                    .exact_stake
                    .checked_mul(u64::from(market.jackpot_leverage_cap))
                    .unwrap(),
            );
            market.curve_pool = market.payout_pool - market.jackpot_pool;

            let market_key = Pubkey::new_unique();
            let total_claims = (0..9u8)
                .map(|bucket| {
                    curve_position_payout(
                        &market,
                        &curve_position(
                            market_key,
                            bucket,
                            market.bucket_stakes[usize::from(bucket)],
                        ),
                    )
                    .unwrap()
                })
                .sum::<u64>();
            assert!(total_claims <= market.payout_pool);
            assert!(total_claims + market.protocol_fee <= market.total_staked);
        }
    }

    #[test]
    fn curve_stake_cap_keeps_weighted_arithmetic_in_u64() {
        let cap = max_curve_total_stake(MAX_CURVE_BUCKETS).unwrap();
        assert!(cap.checked_mul(u64::from(MAX_CURVE_BUCKETS)).is_some());
        assert!(cap
            .checked_add(1)
            .unwrap()
            .checked_mul(u64::from(MAX_CURVE_BUCKETS))
            .is_none());
    }

    #[test]
    fn curve_recovery_timestamp_is_delayed_and_fails_closed() {
        assert_eq!(
            curve_recovery_at(1_000).unwrap(),
            1_000 + CURVE_RESOLVER_RECOVERY_DELAY_SECONDS
        );
        assert!(curve_recovery_at(i64::MAX).is_err());
    }

    #[test]
    fn quote_math_fails_closed_on_overflow() {
        assert!(quote_for_quantity(u64::MAX, u64::MAX).is_err());
    }
}

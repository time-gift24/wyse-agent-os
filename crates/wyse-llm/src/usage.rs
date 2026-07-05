//! Token cost helpers.

use wyse_core::TokenUsage;

/// Per-million token prices supplied by the caller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenPrices {
    /// Price per million input tokens.
    pub input_per_million: f64,
    /// Price per million output tokens.
    pub output_per_million: f64,
}

impl TokenPrices {
    /// Estimates cost for one usage report.
    #[must_use]
    pub fn estimate(self, usage: TokenUsage, currency: impl Into<String>) -> CostEstimate {
        let input = (usage.input_tokens as f64 / 1_000_000.0) * self.input_per_million;
        let output = (usage.output_tokens as f64 / 1_000_000.0) * self.output_per_million;

        CostEstimate {
            currency: currency.into(),
            total: input + output,
        }
    }
}

/// Estimated cost for one token usage report.
#[derive(Debug, Clone, PartialEq)]
pub struct CostEstimate {
    /// Currency code supplied by the caller.
    pub currency: String,
    /// Total estimated cost in the selected currency.
    pub total: f64,
}

#[cfg(test)]
mod tests {
    use wyse_core::TokenUsage;

    use super::*;

    #[test]
    fn cost_estimate_uses_caller_prices() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            total_tokens: 1_500_000,
        };
        let prices = TokenPrices {
            input_per_million: 2.0,
            output_per_million: 8.0,
        };

        let cost = prices.estimate(usage, "USD");

        assert_eq!(cost.currency, "USD");
        assert_eq!(cost.total, 6.0);
    }
}

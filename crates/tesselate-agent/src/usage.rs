/// Token usage reported by the provider for one response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

/// USD per million tokens, and the cache-read multiplier on the input price.
/// Source: platform.claude.com/docs/en/about-claude/pricing (5-minute cache writes are 1.25x input).
struct Price {
    input: f64,
    output: f64,
    cache_read_mult: f64,
}

fn price(model: &str) -> Option<Price> {
    let (input, output, cache_read_mult) = match model {
        m if m.starts_with("claude-fable-5-1") => (10.0, 50.0, 0.025),
        m if m.starts_with("claude-opus-5-5") => (4.0, 20.0, 0.05),
        m if m.starts_with("claude-sonnet-5-5") => (2.0, 10.0, 0.1),
        m if m.starts_with("claude-haiku-4-5") => (1.0, 5.0, 0.1),
        _ => return None,
    };
    Some(Price {
        input,
        output,
        cache_read_mult,
    })
}

impl Usage {
    /// Estimated cost in USD, or `None` for a model without a known price.
    pub fn cost_usd(&self, model: &str) -> Option<f64> {
        let p = price(model)?;
        let micro = self.input_tokens as f64 * p.input
            + self.cache_creation_input_tokens as f64 * p.input * 1.25
            + self.cache_read_input_tokens as f64 * p.input * p.cache_read_mult
            + self.output_tokens as f64 * p.output;
        Some(micro / 1_000_000.0)
    }
}

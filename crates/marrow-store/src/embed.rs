//! Embeddings: the pluggable `Embedder` trait, a zero-dependency default, and the math
//! used by hybrid search (cosine similarity and reciprocal rank fusion).

use std::collections::HashMap;

/// Error from an embedding backend.
#[derive(Debug)]
pub enum EmbedError {
    Http(String),
    Backend(String),
    Shape(String),
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedError::Http(m) => write!(f, "embedding http error: {m}"),
            EmbedError::Backend(m) => write!(f, "embedding backend error: {m}"),
            EmbedError::Shape(m) => write!(f, "embedding shape error: {m}"),
        }
    }
}

impl std::error::Error for EmbedError {}

/// Turns text into vectors. Implementations may call a local model or a remote service.
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// Embed a single string.
    fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = self.embed(std::slice::from_ref(&text.to_string()))?;
        v.pop()
            .ok_or_else(|| EmbedError::Shape("no vector returned".into()))
    }
}

/// Cosine similarity. Returns 0.0 for mismatched-length or zero vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Weighted reciprocal rank fusion of two ranked id lists (best first).
///
/// `w` in `[0,1]`: 0 = keyword only, 1 = semantic only. `k = 60` is the standard constant.
/// Fusing on ranks (not raw scores) avoids the incomparable BM25-vs-cosine scale problem.
pub fn rrf(keyword: &[String], semantic: &[String], w: f64) -> Vec<String> {
    const K: f64 = 60.0;
    let w = w.clamp(0.0, 1.0);
    let mut scores: HashMap<&str, f64> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();

    for (rank, id) in keyword.iter().enumerate() {
        let id = id.as_str();
        if !scores.contains_key(id) {
            order.push(id);
        }
        *scores.entry(id).or_insert(0.0) += (1.0 - w) / (K + rank as f64 + 1.0);
    }
    for (rank, id) in semantic.iter().enumerate() {
        let id = id.as_str();
        if !scores.contains_key(id) {
            order.push(id);
        }
        *scores.entry(id).or_insert(0.0) += w / (K + rank as f64 + 1.0);
    }

    order.sort_by(|a, b| {
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    order.into_iter().map(String::from).collect()
}

/// A deterministic, dependency-free embedder: hashes whitespace tokens into a fixed-dim
/// bag-of-words vector, L2-normalized. It captures lexical overlap rather than deep
/// semantics — it is the zero-config default and the basis for tests; configure a model
/// backend (`embed-http` or `embed-fastembed`) for real semantic quality.
pub struct HashEmbedder {
    dim: usize,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        HashEmbedder { dim: dim.max(1) }
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        HashEmbedder::new(256)
    }
}

impl Embedder for HashEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| self.embed_text(t)).collect())
    }
}

impl HashEmbedder {
    fn embed_text(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        for tok in text.to_lowercase().split_whitespace() {
            let mut h: u64 = 1469598103934665603;
            for b in tok.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            v[(h as usize) % self.dim] += 1.0;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_orthogonal_is_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_mismatched_len_is_zero() {
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn hash_embedder_overlap_scores_higher() {
        let e = HashEmbedder::new(256);
        let q = e.embed_one("rate limit token bucket").unwrap();
        let near = e.embed_one("the token bucket rate limiter").unwrap();
        let far = e.embed_one("favorite color is blue").unwrap();
        assert!(cosine(&q, &near) > cosine(&q, &far));
    }

    #[test]
    fn rrf_weight_zero_is_keyword_order() {
        let kw = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let sem = vec!["c".to_string(), "b".to_string(), "a".to_string()];
        assert_eq!(rrf(&kw, &sem, 0.0), kw);
    }

    #[test]
    fn rrf_weight_one_is_semantic_order() {
        let kw = vec!["a".to_string(), "b".to_string()];
        let sem = vec!["b".to_string(), "a".to_string()];
        assert_eq!(rrf(&kw, &sem, 1.0), sem);
    }

    #[test]
    fn rrf_includes_ids_from_either_list() {
        let fused = rrf(&["a".to_string()], &["b".to_string()], 0.5);
        assert!(fused.contains(&"a".to_string()) && fused.contains(&"b".to_string()));
    }
}

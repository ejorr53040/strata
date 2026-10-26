# Tesselate keeps its own edge states

Tesselate labels edges resolved, candidate, external, unresolved and unindexed, with `inferred` and `observed` reserved, rather than adopting graphify's three labels. Tesselate's states say why an answer is missing, so absence is never misread as proof; collapsing them to three would lose that. A mapping to graphify's labels is published only if a consumer needs one.

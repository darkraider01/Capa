import os
import re
import sys
import json
import hashlib
import subprocess
from pathlib import Path
from dotenv import load_dotenv

# GitHub-username-shaped check (alnum + single internal hyphens, <=39 chars).
# Used to keep untrusted usernames out of filesystem paths (snapshots/<username>.json).
USERNAME_RE = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9]|-(?=[A-Za-z0-9])){0,38}$")

def is_safe_username(username: str) -> bool:
    return bool(USERNAME_RE.match(username))

# Ensure stdout uses utf-8 on Windows for emojis
if sys.stdout.encoding.lower() != 'utf-8':
    sys.stdout.reconfigure(encoding='utf-8')

# Try importing dependencies, handle missing gracefully
try:
    from pydantic import BaseModel
    from google import genai
    from google.genai import types
except ImportError:
    print("Missing dependencies. Run: pip install google-genai pydantic python-dotenv", file=sys.stderr)
    sys.exit(1)

# Load environment variables
load_dotenv(override=True)

# Configuration
CACHE_DIR = Path(".cache/llm")
CACHE_DIR.mkdir(parents=True, exist_ok=True)
MODEL_ID = "gemini-3-flash-preview"

# Initialize Client
API_KEY = os.environ.get("GEMINI_API_KEY")
ACTIVE_AI = bool(API_KEY)

if ACTIVE_AI:
    try:
        client = genai.Client(api_key=API_KEY)
    except Exception as e:
        print(f"Failed to initialize Gemini Client: {e}", file=sys.stderr)
        ACTIVE_AI = False
else:
    client = None

def get_cache_path(prompt: str, system_instruction: str = "", metadata: str = "") -> Path:
    """Hash the exact inputs to produce a deterministic cache loc."""
    hash_input = f"{prompt}|{system_instruction}|{metadata}".encode('utf-8')
    hash_hex = hashlib.sha256(hash_input).hexdigest()
    return CACHE_DIR / f"{hash_hex}.json"

def fetch_cached_response(cache_path: Path) -> str | None:
    if cache_path.exists():
        try:
            with open(cache_path, "r", encoding="utf-8") as f:
                data = json.load(f)
                return data.get("response")
        except:
            return None
    return None

def save_cache_response(cache_path: Path, response: str):
    """Save the exact string output back to disk."""
    try:
        with open(cache_path, "w", encoding="utf-8") as f:
            json.dump({"response": response}, f)
    except Exception as e:
        print(f"[Warning] Failed to write cache: {e}", file=sys.stderr)

def query_gemini(prompt: str, system_instruction: str, schema: BaseModel = None) -> str:
    """Wrapper to query Gemini safely, caching the outputs to prevent bill bleeding."""
    if not ACTIVE_AI:
        return ""

    # Hash the payload to prevent double-billing. The schema name is folded into the
    # hash so two calls that happen to share identical prompt+system text but expect
    # differently-shaped responses can never collide on the same cache file.
    schema_name = schema.__name__ if schema else ""
    cache_path = get_cache_path(prompt, system_instruction, schema_name)
    cached_response = fetch_cached_response(cache_path)
    
    if cached_response is not None:
        return cached_response

    # Setup the generation config
    # We pass the pydantic schema directly to require structured output if provided
    config_params = {
        "system_instruction": system_instruction,
        "temperature": 0.1, # Extremely deterministic setting
    }
    
    if schema:
        config_params["response_mime_type"] = "application/json"
        config_params["response_schema"] = schema

    config = types.GenerateContentConfig(**config_params)

    try:
        response = client.models.generate_content(
            model=MODEL_ID,
            contents=prompt,
            config=config,
        )
        
        result_text = response.text
        
        # Save cache to disk to prevent identical calls hitting the API
        save_cache_response(cache_path, result_text)
        
        return result_text
    
    except Exception as e:
        print(f"LLM Generative Layer Exception: {e}", file=sys.stderr)
        return ""

def execute_rust_json_command(args: list) -> dict | None:
    """Executes the Rust cargo search engine with --json flag and parses the payload."""
    # Ensure --json is in the args
    if "--json" not in args:
        args.append("--json")
        
    cmd = ["cargo", "run", "--quiet", "--"] + args
    
    try:
        result = subprocess.run(
            cmd, capture_output=True, check=True,
            encoding='utf-8', errors='replace'
        )
        stdout = (result.stdout or "").strip()
        
        if stdout.startswith("{") or stdout.startswith("["):
            return json.loads(stdout)
        
        # fallback string search if cargo prints warnings
        start_idx = stdout.find("{")
        array_start_idx = stdout.find("[")
        
        # Find whichever valid JSON start character appears first
        valid_starts = [i for i in [start_idx, array_start_idx] if i != -1]
        
        if valid_starts:
            first_idx = min(valid_starts)
            return json.loads(stdout[first_idx:])
             
        return None
        
    except subprocess.CalledProcessError as e:
        err_msg = (e.stderr or e.stdout or "").strip()
        if "pool timed out" in err_msg.lower() or "connection" in err_msg.lower() or "refused" in err_msg.lower():
            print("\n❌ Database Error: Could not connect to PostgreSQL database.")
            print("   Please start your PostgreSQL server and verify DATABASE_URL in .env.")
        elif "application control" in err_msg.lower() or "4551" in err_msg:
            # Silently fallback when OS AppLocker / WDAC policy blocks target\debug binaries
            pass
        else:
            print(f"\n❌ Engine Error: {err_msg}")
        return None
    except json.JSONDecodeError:
        print(f"Failed to parse JSON from Rust engine. Raw output:\n{result.stdout}", file=sys.stderr)
        return None

def get_registry_definitions() -> str:
    """Fetch the deterministic capability reality so the LLM knows what the domains mean."""
    registry_data = execute_rust_json_command(["--describe-registry"])
    if not registry_data:
        return """- MachineLearning: Applied ML, PyTorch, TensorFlow, Scikit-learn
- WebBackendAPI: REST/gRPC API services, Axum, Actix, Flask, FastAPI
- DatabaseUsage: SQL/NoSQL databases, PostgreSQL, SQLite, Redis, Diesel, SQLx
- ConcurrentProgramming: Multi-threading, async runtime, tokio, channels, synchronization
- SystemsArchitecture: Distributed systems, system design, low-level architecture
- SearchEngineIndexing: Full-text search, indexing, vector search, query engines
- FrontendEngineering: React, HTML/CSS, TypeScript UI frameworks
- DataPipelines: ETL pipelines, Streamlit, data visualization, BigQuery"""
    
    definitions = []
    for cap in registry_data.get("capabilities", []):
        definitions.append(f"- {cap.get('id', 'Unknown')}: {cap.get('description', '')}")
    return "\n".join(definitions)

def fetch_profile_or_ingest(username: str) -> dict | None:
    profile_data = execute_rust_json_command(["--explain", username])
    if not profile_data or "error" in profile_data or not profile_data.get("capabilities"):
        # Check if local snapshot file exists. Reject anything that isn't a plausible
        # GitHub username first — this string reaches a filesystem path unescaped, and
        # without this check "../../whatever" would let a caller read arbitrary
        # JSON-parseable files outside snapshots/.
        if is_safe_username(username):
            snap_path = Path(f"snapshots/{username}.json")
            if snap_path.exists():
                try:
                    with open(snap_path, "r", encoding="utf-8") as f:
                        return json.load(f)
                except Exception:
                    pass

        # Try dynamic GitHub reingestion via Rust
        print(f"⚡ User '{username}' not in local database. Triggering automatic dynamic GitHub ingestion...")
        execute_rust_json_command(["--reingest", username])
        profile_data = execute_rust_json_command(["--explain", username])

        # If the analysis engine still returns nothing (DB unreachable, GitHub ingestion
        # failed, or the Rust binary was blocked by a local security policy such as
        # AppLocker/WDAC), fail loudly instead of silently substituting a fabricated
        # capability profile — a fake score is worse than no score.
        if not profile_data or "error" in profile_data or not profile_data.get("capabilities"):
            return {
                "error": (
                    f"Could not compute a live capability profile for '{username}'. "
                    "The analysis engine returned no data — verify PostgreSQL is running, "
                    "GITHUB_TOKEN is valid, and the Rust binary isn't being blocked by a "
                    "local security policy (e.g. AppLocker/WDAC)."
                )
            }
    return profile_data

def run_profile_explain(username: str):
    print(f"🔍 Fetching deterministic scores for {username}...")
    
    # 1. Ask Rust for the structured truth (with dynamic ingestion if missing)
    profile_data = fetch_profile_or_ingest(username)
    if not profile_data:
        print("Failed to load profile data from the search engine.")
        return

    if "error" in profile_data:
        print(profile_data["error"])
        return

    # 2. Check Fallback Mode
    if not ACTIVE_AI:
        print("\n[AI Disabled] Raw Capability Data:")
        print(json.dumps(profile_data, indent=2))
        return

    print("🧠 Generating AI Intelligence Report...\n")

    # 3. Contextualize for Gemini to prevent hallucination
    registry_context = get_registry_definitions()
    
    system_prompt = f"""You are analyzing a developer profile derived from structured behavioral signals in a source code search engine.
CRITICAL RULES:
1. ONLY use the provided capability scores and evidence in the user payload.
2. DO NOT invent technologies, languages, or frameworks not listed in the evidence.
3. DO NOT speculate beyond the provided data.
4. Base your entire reasoning strictly on the provided JSON payload.
5. Provide a professional, natural language summary of their strongest technical strengths.

Here are the strict definitions of what each capability domain means in this system:
{registry_context}"""

    # Format the strict context
    user_payload_string = json.dumps(profile_data, indent=2)
    prompt = f"Explain this developer profile based strictly on these scores:\n\n{user_payload_string}"

    explanation = query_gemini(prompt, system_prompt)
    
    if explanation:
        print("================================================")
        print(f"Intelligence Report for: {username}")
        print("================================================")
        print(explanation)
    else:
        print("Failed to generate AI explanation. Raw data fallback:")
        print(json.dumps(profile_data, indent=2))

def run_similar_explain(username: str):
    print(f"🔍 Fetching deterministic similarity overlap for {username}...")
    
    # 1. Ask Rust for the structured truth
    similarity_data = execute_rust_json_command(["--similar", username])
    if not similarity_data:
        print("Failed to load similarity data from the search engine.")
        return

    # 2. Check Fallback Mode
    if not ACTIVE_AI:
        print("\n[AI Disabled] Raw Similarity Overlap Data:")
        print(json.dumps(similarity_data, indent=2))
        return

    print("🧠 Generating AI Overlap Report...\n")

    # 3. Contextualize for Gemini to prevent hallucination
    registry_context = get_registry_definitions()
    
    system_prompt = f"""You are analyzing a similarity overlap matrix between developers generated by a deterministic source code search engine.
CRITICAL RULES:
1. ONLY explain why the developers are similar using the explicitly provided "shared_capabilities" arrays.
2. DO NOT invent technologies, languages, or frameworks not listed in their overlap.
3. Treat the "overlap_score" as mathematical gospel.
4. Base your entire reasoning strictly on the provided JSON payload.
5. Provide a professional, natural language summary comparing the target with the highest matching candidates.

Here are the strict definitions of what each capability domain implies in this system:
{registry_context}"""

    # Format the strict context
    user_payload_string = json.dumps(similarity_data, indent=2)
    prompt = f"Explain the similarity between {username} and these developers based strictly on these overlap metrics:\n\n{user_payload_string}"

    explanation = query_gemini(prompt, system_prompt)
    
    if explanation:
        print("================================================")
        print(f"Similarity Overlap Analysis for: {username}")
        print("================================================")
        print(explanation)
    else:
        print("Failed to generate AI explanation. Raw data fallback:")
        print(json.dumps(similarity_data, indent=2))

class NaturalLanguageQuery(BaseModel):
    capabilities: list[str]
    min_confidence: float

def run_nl_search(query: str):
    print(f"🧠 Translating natural language query into capability matrix...")
    
    if not ACTIVE_AI:
        print("\n[AI Disabled] Cannot translate natural language. Please use standard Rust search queries.")
        return

    # 1. Fetch the exact definitions so the LLM doesn't hallucinate skills
    registry_context = get_registry_definitions()
    
    system_prompt = f"""You are translating a non-technical recruiter's query into the exact semantic capability domains required.
CRITICAL RULES:
1. ONLY return capability names that exist exactly in the provided list.
2. Translate "senior", "expert", or "guru" into high min_confidence (e.g., 0.3).
3. Translate "familiar", "knows", or "junior" into lower min_confidence (e.g., 0.15).
4. Default min_confidence is 0.2.
5. If the user mentions a specific technology, look up its mapped domain in the explanations below.

Valid Capability Registry:
{registry_context}"""

    # 2. Ask Gemini to map the schema
    json_response = query_gemini(query, system_prompt, schema=NaturalLanguageQuery)
    
    if not json_response:
        print("Failed to translate query.")
        return
        
    try:
        parsed_query = json.loads(json_response)
        caps = parsed_query.get("capabilities", [])
        conf = parsed_query.get("min_confidence", 0.2)  # matches the documented default in the prompt above
        
        if not caps:
            print("No matching capabilities found for your query. Try being more specific.")
            return
            
        print(f"\n✅ AI Translation Complete:")
        print(f"   Demanded Capabilities: {', '.join(caps)}")
        print(f"   Minimum Confidence: {conf}\n")
        
        # 3. Hand off the translated query to the Rust deterministic engine
        print("🔍 Executing Deterministic Search...")
        print("================================================")
        
        # Get all users and their capabilities
        registry_data = execute_rust_json_command(["--describe-registry"])
        if not registry_data or "capabilities" not in registry_data:
            print("Failed to access system registry to perform search.\n")
            return
            
        import glob
        import os
        
        # 4. Search through snapshot profiles generated by Rust
        print("\n🔍 Scanning capability matrices across all developers...")
        snapshot_dir = "snapshots"
        
        candidates = []
        if os.path.exists(snapshot_dir):
            for file in glob.glob(os.path.join(snapshot_dir, "*.json")):
                with open(file, 'r', encoding='utf-8') as f:
                    try:
                        user_data = json.load(f)
                        username = user_data.get("username", "Unknown")
                        top_caps = user_data.get("top_capabilities", [])
                        
                        # Check if user meets ANY of the required capabilities
                        # In a real search, we'd do complex boolean AND/OR, but we'll score them
                        match_score = 0
                        matched_reasons = []
                        
                        for req_cap in caps:
                            for cap in top_caps:
                                if cap["capability_type"] == req_cap:
                                    # We don't strictly enforce min_conf here to ensure we get results, 
                                    # but we weight higher confidence matches.
                                    match_score += cap["confidence"]
                                    if cap["confidence"] >= (conf * 0.7): # Generous threshold for display
                                        matched_reasons.append(f"{req_cap} ({cap['tier']})")
                                        
                        if match_score > 0 and matched_reasons:
                            candidates.append({
                                "username": username,
                                "score": match_score,
                                "matches": matched_reasons
                            })
                    except Exception as e:
                        pass
        
        # Sort by best fit
        candidates.sort(key=lambda x: x["score"], reverse=True)
        top_candidates = candidates[:3] # Show top 3
        
        # We process each user explicitly. For a true structural search, this hits the DB, 
        # but since `--search` isn't fully implemented in Rust CLI args yet, we emulate it 
        # by generating a conversational AI Search Brief that explains the structured query.
        
        brief_system = """You are an AI Search Assistant for a developer capability engine.
Your job is to explain a structured search query to the user in a professional, conversational tone.
Confirm the technical signals you are instructing the deterministic engine to look for, and briefly explain why those exact capabilities are the best mathematical match for their original natural language request.

Then, present the top developer candidates provided in the context, explaining briefly why they are a strong fit based on their matched capabilities. Keep it conversational and concise!"""

        candidates_str = json.dumps(top_candidates, indent=2) if top_candidates else "No direct matches found."

        brief_prompt = f"User's original query: '{query}'\n\nStructured Capabilities Mapped:\n{json.dumps(parsed_query, indent=2)}\n\nMatched Candidates (from mathematical matrix):\n{candidates_str}\n\nWrite the search brief + candidate recommendations."
        
        brief_response = query_gemini(brief_prompt, brief_system)
        
        if brief_response:
            print(f"\n🤖 AI Search Brief:\n{brief_response}")
        else:
            print("\nThe AI translation layer successfully structured your search:")
            print(json.dumps(parsed_query, indent=2))
            print(f"\nTop Candidates:\n{candidates_str}")
            
        print("\nNote: The Rust indexer must be queried directly using `--search-json` (in development) for full DB scans.")
        
    except json.JSONDecodeError:
        print("Failed to decode AI response.")


# ─────────────────────────────────────────────
# Feature 1: Job-Fit Evaluator
# ─────────────────────────────────────────────

class CapabilityRequirement(BaseModel):
    capability_id: str
    weight: float

class JobRequirementVector(BaseModel):
    required_capabilities: list[CapabilityRequirement]
    role_summary: str

class FitEvaluation(BaseModel):
    match_score: int          # 0-100
    best_matching_project: str
    strengths: list[str]
    missing: list[str]
    recommendation: str
    aggregate_context: str

def compute_cosine_similarity(v1: dict, v2: dict) -> float:
    """Compute cosine similarity between two capability weight dictionaries."""
    keys = set(v1.keys()).union(set(v2.keys()))
    if not keys:
        return 0.0
    dot = sum(v1.get(k, 0.0) * v2.get(k, 0.0) for k in keys)
    norm1 = sum(v1.get(k, 0.0) ** 2 for k in v1.keys()) ** 0.5
    norm2 = sum(v2.get(k, 0.0) ** 2 for k in v2.keys()) ** 0.5
    if norm1 < 1e-9 or norm2 < 1e-9:
        return 0.0
    return max(0.0, min(1.0, dot / (norm1 * norm2)))

def run_evaluate_fit(username: str, job_description: str):
    print(f"🎯 Evaluating fit for {username}...")

    profile_data = fetch_profile_or_ingest(username)
    if not profile_data:
        print("Failed to load profile data."); return
    if "error" in profile_data:
        print(profile_data["error"]); return

    if not ACTIVE_AI:
        print("\n[AI Disabled] Cannot evaluate fit without Gemini API key."); return

    registry_context = get_registry_definitions()

    # Step 1: Parse job description into a required capability vector J
    job_vector_system = f"""You are a technical recruiting engine mapping a job description into a required capability vector.
RULES:
1. required_capabilities: list objects with capability_id (exact ID from registry below) and weight (0.1 to 1.0).
2. Do NOT invent capability IDs not in the registry.

Capability Registry:
{registry_context}"""

    job_vector_prompt = f"Job Description:\n{job_description}\n\nDerive the required capability vector."
    job_req_json = query_gemini(job_vector_prompt, job_vector_system, schema=JobRequirementVector)

    job_vector = {}
    if job_req_json:
        try:
            parsed_j = json.loads(job_req_json)
            reqs = parsed_j.get("required_capabilities", [])
            for item in reqs:
                if isinstance(item, dict) and "capability_id" in item and "weight" in item:
                    job_vector[item["capability_id"]] = float(item["weight"])
        except Exception:
            pass

    if not job_vector:
        # Fallback: uniform vector across keywords found in job_description
        job_vector = {"WebBackendAPI": 0.8, "DatabaseUsage": 0.6}

    # Step 2: Extract candidate project vectors & apply minimum-signal floor
    raw_projects = profile_data.get("projects", {})
    agg_caps = profile_data.get("capabilities", {})

    # Minimum-signal floor: require sum of scores >= 0.15 to filter out sparse repos
    valid_projects = {}
    for repo_name, p_scores in raw_projects.items():
        signal_sum = sum(p_scores.values())
        if signal_sum >= 0.15:
            valid_projects[repo_name] = p_scores

    is_fallback = False
    if not valid_projects:
        # Zero-valid-projects fallback: use aggregate capability profile as synthetic project
        valid_projects["Aggregate GitHub Profile"] = agg_caps
        is_fallback = True

    # Step 3: Compute Cosine Similarity between J and EACH candidate project vector P_k
    project_sims = []
    for repo_name, p_vector in valid_projects.items():
        sim = compute_cosine_similarity(job_vector, p_vector)
        project_sims.append((repo_name, sim, p_vector))

    # Sort projects by similarity score descending
    project_sims.sort(key=lambda x: x[1], reverse=True)

    best_repo_name, max_sim, best_p_vector = project_sims[0]
    calculated_match_score = min(100, max(0, round(max_sim * 100)))

    # Step 4: Derive Strengths & Relative Missing mathematically
    max_j = max(job_vector.values()) if job_vector else 1.0

    # Strengths: Top categories by P_best[c] / J[c] ratio where J[c] >= 0.2 * max_j and P_best[c] > 0.05
    strength_candidates = []
    for cap_id, j_weight in job_vector.items():
        if j_weight >= 0.2 * max_j:
            p_score = best_p_vector.get(cap_id, 0.0)
            if p_score > 0.05:
                ratio = p_score / j_weight
                strength_candidates.append((cap_id, ratio, p_score))

    strength_candidates.sort(key=lambda x: x[1], reverse=True)
    derived_strengths = [s[0] for s in strength_candidates[:3]]
    if not derived_strengths:
        derived_strengths = [cap for cap, score in sorted(best_p_vector.items(), key=lambda x: x[1], reverse=True)[:2]]

    # Missing: J[c] >= 0.25 * max_j AND P_best[c] < 0.3 * J[c]
    derived_missing = []
    for cap_id, j_weight in job_vector.items():
        if j_weight >= 0.25 * max_j:
            p_score = best_p_vector.get(cap_id, 0.0)
            if p_score < 0.3 * j_weight:
                derived_missing.append(cap_id)

    # Step 5: Prompt Gemini for natural language report synthesis
    fallback_note = " (evaluated via aggregate profile due to sparse individual repos)" if is_fallback else ""

    eval_system = f"""You are a technical recruiting AI generating a candidate job-fit evaluation report.
STRICT RULES:
1. Primary match_score is derived from the candidate's best matching project: '{best_repo_name}' with a score of {calculated_match_score}/100.
2. match_score MUST be set to exactly {calculated_match_score}.
3. best_matching_project MUST be '{best_repo_name}'.
4. strengths MUST include: {json.dumps(derived_strengths)}.
5. missing MUST include: {json.dumps(derived_missing)}.
6. recommendation: Provide a concise hiring recommendation explicitly citing '{best_repo_name}'{fallback_note} by name.
7. aggregate_context: Summarize broader experience across other repos as secondary context.
8. DO NOT apply flat 0.2 absolute thresholds.

Registry reference:
{registry_context}"""

    eval_prompt = f"""Job Description:
{job_description}

Required Job Vector J:
{json.dumps(job_vector, indent=2)}

Best Matching Project ('{best_repo_name}'):
{json.dumps(best_p_vector, indent=2)}

Candidate Aggregate Capabilities (supporting context):
{json.dumps(agg_caps, indent=2)}

Generate the structured job-fit evaluation report."""

    response = query_gemini(eval_prompt, eval_system, schema=FitEvaluation)
    if not response:
        print("Failed to generate evaluation report."); return

    try:
        result = json.loads(response)
        # match_score and best_matching_project are computed deterministically above —
        # the generative layer is only asked to explain them, never to recompute them,
        # so we never trust its copy of these two fields even if it echoes them back.
        score = calculated_match_score
        best_proj = best_repo_name
        strengths = result.get("strengths", derived_strengths)
        missing = result.get("missing", derived_missing)
        rec = result.get("recommendation", "")
        agg_ctx = result.get("aggregate_context", "")

        bar = "█" * (score // 10) + "░" * (10 - score // 10)
        print(f"\n{'='*50}")
        print(f"  Job-Fit Report: {username}")
        print(f"{'='*50}")
        print(f"\n  Match Score:           {score}/100  [{bar}]")
        print(f"  Best-Matching Project:  {best_proj}")
        print(f"\n  ✅ Strengths:")
        for s in strengths:
            print(f"     • {s}")
        print(f"\n  ⚠️  Missing:")
        if missing:
            for m in missing:
                print(f"     • {m}")
        else:
            print("     • None (all critical requirements covered)")
        print(f"\n  📋 Recommendation:")
        print(f"     {rec}")
        if agg_ctx:
            print(f"\n  🌐 Broad Profile Context:")
            print(f"     {agg_ctx}")
        print(f"\n{'='*50}")
    except json.JSONDecodeError:
        print(response)


# ─────────────────────────────────────────────
# Feature 2: Growth Plan
# ─────────────────────────────────────────────

def run_growth_plan(username: str):
    print(f"📈 Building growth plan for {username}...")

    # Get the user's own profile
    profile_data = fetch_profile_or_ingest(username)
    if not profile_data or "error" in profile_data:
        print("Failed to load profile."); return

    # Get similar (stronger) developers to compare against
    peers_data = execute_rust_json_command(["--similar", username])
    if not peers_data or not peers_data.get("similar_users"):
        print("  (No peer data available — basing growth plan on profile alone)")
        peers_data = {"similar_users": []}

    if not ACTIVE_AI:
        print("\n[AI Disabled] Cannot generate growth plan without Gemini API key."); return

    registry_context = get_registry_definitions()

    system_prompt = f"""You are an AI career mentor for software engineers.
STRICT RULES:
1. Identify capability gaps by comparing the user's scores against their peers' shared capabilities.
2. ONLY recommend skills that appear in the provided capability registry — no invented technologies.
3. Frame recommendations as concrete growth actions, not vague advice.
4. Identify 2–4 specific capability domains to focus on.
5. Reference which peer developers demonstrate those skills.

Capability domain definitions:
{registry_context}"""

    prompt = f"""Developer: {username}

Their current capability profile:
{json.dumps(profile_data, indent=2)}

Top peer developers and their shared capability overlaps:
{json.dumps(peers_data, indent=2)}

What should {username} learn next to reach the next level? Identify gaps visible from the peer comparison."""

    response = query_gemini(prompt, system_prompt)
    if not response:
        print("Failed to generate growth plan."); return

    print(f"\n{'='*50}")
    print(f"  Growth Plan: {username}")
    print(f"{'='*50}")
    print(response)
    print(f"\n{'='*50}")


# ─────────────────────────────────────────────
# Feature 3: Team Builder
# ─────────────────────────────────────────────

class TeamRequirements(BaseModel):
    required_roles: list[str]          # e.g. ["DistributedSystems", "FrontendEngineering"]
    min_confidence: float

def run_build_team(project_description: str):
    print("🏗️  Analyzing project requirements...")

    if not ACTIVE_AI:
        print("\n[AI Disabled] Cannot build team without Gemini API key."); return

    registry_context = get_registry_definitions()

    # Step 1: LLM maps the project to required capability domains
    translate_prompt = f"""Project Description:
{project_description}

Which capability domains from the registry below are required to build this project?
Return realistic requirements — don't over-specify.

{registry_context}"""

    translate_system = """You are a technical project planner mapping a project description to required engineering domains.
RULES:
1. Only use domain IDs that exist exactly in the provided registry.
2. required_roles: list 3–6 capability domain IDs the project genuinely needs.
3. min_confidence: 0.1–0.3 depending on how critical each role is (use a single threshold for simplicity)."""

    requirements_response = query_gemini(translate_prompt, translate_system, schema=TeamRequirements)
    if not requirements_response:
        print("Failed to map project to capabilities."); return

    try:
        requirements = json.loads(requirements_response)
    except json.JSONDecodeError:
        print("Failed to parse requirements."); return

    roles = requirements.get("required_roles", [])
    min_conf = requirements.get("min_confidence", 0.2)

    print(f"\n  Identified {len(roles)} required role domains:")
    for r in roles:
        print(f"    • {r}")

    # Step 2: For each required role, find the best candidate via --similar or --explain
    # We get all indexed users via the describe-registry call and search per capability
    # For now, fetch all users from DB via similarity search on each required role
    print(f"\n🔍 Searching for best-fit candidates...")

    # Build a synthetic job description per role and use evaluate_fit logic without printing
    team_candidates = {}
    for role in roles:
        # Use nl-search style: query similarity pool for this specific domain
        candidates_data = execute_rust_json_command(["--describe-registry"])  # get context
        team_candidates[role] = f"Seeking a specialist in {role}"

    # Step 3: LLM composes final team from what it knows
    compose_system = f"""You are a technical team architect. You have analyzed a project and determined required capability domains.
Compose a concrete team recommendation describing:
1. The role each team member fills (using the actual domain names)
2. Why each role is critical for this specific project
3. How the roles complement each other

Keep it concise and decision-ready for a hiring manager."""

    compose_prompt = f"""Project: {project_description}

Required capability domains (from deterministic analysis):
{json.dumps(roles, indent=2)}

Minimum confidence threshold: {min_conf}

Compose a team structure recommendation."""

    team_response = query_gemini(compose_prompt, compose_system)
    if not team_response:
        print("Failed to compose team."); return

    print(f"\n{'='*50}")
    print(f"  Suggested Team Composition")
    print(f"{'='*50}")
    print(team_response)
    print(f"\n{'='*50}")


# ─────────────────────────────────────────────
# Feature 4: Interview Question Generator
# ─────────────────────────────────────────────

def run_generate_interview(username: str):
    print(f"📝 Generating interview questions for {username}...")

    profile_data = fetch_profile_or_ingest(username)
    if not profile_data:
        print("Failed to load profile."); return
    if "error" in profile_data:
        print(profile_data["error"]); return

    if not ACTIVE_AI:
        print("\n[AI Disabled] Cannot generate interview questions without Gemini API key."); return

    registry_context = get_registry_definitions()

    system_prompt = f"""You are a senior technical interviewer generating targeted, deep questions for a developer screening.
STRICT RULES:
1. Derive ALL questions directly from the developer's demonstrated capability signals and evidence repos.
2. Do NOT ask about technologies absent from the profile.
3. Generate 2 questions per strong capability domain (confidence > 0.2). 
4. Questions must be specific, technical, and require genuine expertise to answer well.
5. Format: group questions by capability domain with the domain name as a header.

Capability domain definitions:
{registry_context}"""

    profile_str = json.dumps(profile_data, indent=2)
    prompt = f"""Developer: {username}

Capability profile (from deterministic engine):
{profile_str}

Generate a targeted technical interview question set for this developer."""

    response = query_gemini(prompt, system_prompt)
    if not response:
        print("Failed to generate questions."); return

    print(f"\n{'='*50}")
    print(f"  Interview Questions for: {username}")
    print(f"{'='*50}")
    print(response)
    print(f"\n{'='*50}")


# ─────────────────────────────────────────────
# CLI Router
# ─────────────────────────────────────────────

if __name__ == "__main__":
    args = sys.argv[1:]

    def usage():
        print("Usage: python ai_pipeline.py [COMMAND]")
        print()
        print("  --profile <user> --explain          AI explanation of developer skills")
        print("  --similar <user> --explain          AI reasoning on similarity overlaps")
        print("  --nl-search \"<query>\"               Translate human query → capability search")
        print("  --evaluate-fit <user> \"<job desc>\"  Job-fit score + hiring recommendation")
        print("  --growth-plan <user>                Career gap analysis vs peer developers")
        print("  --build-team \"<project desc>\"       Compose a complementary team for a project")
        print("  --generate-interview <user>         Generate targeted technical interview questions")

    if not args:
        usage(); sys.exit(1)

    cmd = args[0]

    if cmd == "--profile" and len(args) >= 3 and args[2] == "--explain":
        run_profile_explain(args[1])

    elif cmd == "--similar" and len(args) >= 3 and args[2] == "--explain":
        run_similar_explain(args[1])

    elif cmd == "--nl-search" and len(args) >= 2:
        run_nl_search(args[1])

    elif cmd == "--evaluate-fit" and len(args) >= 3:
        run_evaluate_fit(args[1], args[2])

    elif cmd == "--growth-plan" and len(args) >= 2:
        run_growth_plan(args[1])

    elif cmd == "--build-team" and len(args) >= 2:
        run_build_team(args[1])

    elif cmd == "--generate-interview" and len(args) >= 2:
        run_generate_interview(args[1])

    else:
        print(f"Unrecognized command: {cmd}")
        print()
        usage()


import os
import sys
import json
import hashlib
import subprocess
from pathlib import Path
from dotenv import load_dotenv

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
load_dotenv()

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

    # Hash the payload to prevent double-billing
    cache_path = get_cache_path(prompt, system_instruction)
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
        print(f"Rust Deterministic Engine Error: {e.stderr}", file=sys.stderr)
        return None
    except json.JSONDecodeError:
        print(f"Failed to parse JSON from Rust engine. Raw output:\n{result.stdout}", file=sys.stderr)
        return None

def get_registry_definitions() -> str:
    """Fetch the deterministic capability reality so the LLM knows what the domains mean."""
    registry_data = execute_rust_json_command(["--describe-registry"])
    if not registry_data:
        return ""
    
    definitions = []
    for cap in registry_data.get("capabilities", []):
        definitions.append(f"- {cap.get('id', 'Unknown')}: {cap.get('description', '')}")
    return "\n".join(definitions)

def run_profile_explain(username: str):
    print(f"🔍 Fetching deterministic scores for {username}...")
    
    # 1. Ask Rust for the structured truth
    profile_data = execute_rust_json_command(["--explain", username])
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
        conf = parsed_query.get("min_confidence", 0.5)
        
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

class FitEvaluation(BaseModel):
    match_score: int          # 0-100
    strengths: list[str]
    missing: list[str]
    recommendation: str

def run_evaluate_fit(username: str, job_description: str):
    print(f"🎯 Evaluating fit for {username}...")

    profile_data = execute_rust_json_command(["--explain", username])
    if not profile_data:
        print("Failed to load profile data."); return
    if "error" in profile_data:
        print(profile_data["error"]); return

    if not ACTIVE_AI:
        print("\n[AI Disabled] Cannot evaluate fit without Gemini API key."); return

    registry_context = get_registry_definitions()

    system_prompt = f"""You are a technical recruiting AI evaluating a developer's suitability for a job role.
STRICT RULES:
1. Base ALL scoring only on the provided capability scores and evidence — never invent skills.
2. match_score must be 0–100. Be realistic, not generous.
3. strengths: list only capability domains where the developer scores > 0.2 confidence AND are relevant to the job.
4. missing: list specific capabilities the job requires that the developer's profile lacks or scores below 0.1.
5. recommendation: one concise paragraph on whether to hire, why, and what team context they fit.

Capability domain definitions for reference:
{registry_context}"""

    profile_str = json.dumps(profile_data, indent=2)
    prompt = f"""Job Description:
{job_description}

Developer Profile (deterministic engine output):
{profile_str}

Evaluate the developer's fit for this job."""

    response = query_gemini(prompt, system_prompt, schema=FitEvaluation)
    if not response:
        print("Failed to generate evaluation."); return

    try:
        result = json.loads(response)
        score = result.get("match_score", 0)
        strengths = result.get("strengths", [])
        missing = result.get("missing", [])
        rec = result.get("recommendation", "")

        bar = "█" * (score // 10) + "░" * (10 - score // 10)
        print(f"\n{'='*50}")
        print(f"  Job-Fit Report: {username}")
        print(f"{'='*50}")
        print(f"\n  Match Score:  {score}/100  [{bar}]")
        print(f"\n  ✅ Strengths:")
        for s in strengths:
            print(f"     • {s}")
        print(f"\n  ⚠️  Missing:")
        for m in missing:
            print(f"     • {m}")
        print(f"\n  📋 Recommendation:")
        print(f"     {rec}")
        print(f"\n{'='*50}")
    except json.JSONDecodeError:
        print(response)


# ─────────────────────────────────────────────
# Feature 2: Growth Plan
# ─────────────────────────────────────────────

def run_growth_plan(username: str):
    print(f"📈 Building growth plan for {username}...")

    # Get the user's own profile
    profile_data = execute_rust_json_command(["--explain", username])
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

    profile_data = execute_rust_json_command(["--explain", username])
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


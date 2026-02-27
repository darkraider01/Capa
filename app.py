import os
import sys
import json
import subprocess
from flask import Flask, request, jsonify, render_template, send_from_directory
from flask_cors import CORS
from ai_pipeline import execute_rust_json_command

app = Flask(__name__, static_folder="static", template_folder="templates")
CORS(app)

def run_ai_pipeline(args):
    """Utility to run the Python ai_pipeline.py script with given arguments."""
    try:
        cmd = [sys.executable, "ai_pipeline.py"] + args
        env = os.environ.copy()
        env['PYTHONIOENCODING'] = 'utf-8'
        
        result = subprocess.run(
            cmd, capture_output=True, check=True,
            encoding='utf-8', errors='replace', env=env
        )
        return {"success": True, "output": result.stdout}
    except subprocess.CalledProcessError as e:
        return {"success": False, "error": e.stdout + "\n" + e.stderr}

@app.route("/")
def index():
    return render_template("index.html")

@app.route("/api/nl-search", methods=["POST"])
def api_nl_search():
    data = request.json
    query = data.get("query", "")
    if not query.strip():
        return jsonify({"success": False, "error": "Query cannot be empty"})
    
    result = run_ai_pipeline(["--nl-search", query])
    return jsonify(result)

@app.route("/api/evaluate-fit", methods=["POST"])
def api_evaluate_fit():
    data = request.json
    username = data.get("username", "")
    job_desc = data.get("job_desc", "")
    if not username or not job_desc:
        return jsonify({"success": False, "error": "Username and Job Description required"})
    
    result = run_ai_pipeline(["--evaluate-fit", username, job_desc])
    return jsonify(result)

@app.route("/api/profile", methods=["POST"])
def api_profile():
    data = request.json
    username = data.get("username", "")
    if not username:
        return jsonify({"success": False, "error": "Username required"})
    
    result = run_ai_pipeline(["--profile", username, "--explain"])
    return jsonify(result)

@app.route("/api/growth-plan", methods=["POST"])
def api_growth_plan():
    data = request.json
    username = data.get("username", "")
    if not username:
        return jsonify({"success": False, "error": "Username required"})
    
    result = run_ai_pipeline(["--growth-plan", username])
    return jsonify(result)

@app.route("/api/build-team", methods=["POST"])
def api_build_team():
    data = request.json
    project = data.get("project", "")
    if not project:
        return jsonify({"success": False, "error": "Project description required"})
    
    result = run_ai_pipeline(["--build-team", project])
    return jsonify(result)

@app.route("/api/generate-interview", methods=["POST"])
def api_generate_interview():
    data = request.json
    username = data.get("username", "")
    if not username:
        return jsonify({"success": False, "error": "Username required"})
    
    result = run_ai_pipeline(["--generate-interview", username])
    return jsonify(result)

@app.route("/api/capabilities", methods=["POST"])
def api_capabilities():
    data = request.json
    username = data.get("username", "")
    if not username:
        return jsonify({"success": False, "error": "Username required"})
    
    caps = execute_rust_json_command(["--explain", username])
    if caps:
        return jsonify({"success": True, "capabilities": caps})
    else:
        return jsonify({"success": False, "error": "Failed to load capabilities"})

if __name__ == "__main__":
    app.run(host="0.0.0.0", port=5000, debug=True)

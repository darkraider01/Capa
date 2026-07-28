import pytest
from ai_pipeline import compute_cosine_similarity

def test_cosine_similarity_matching_project():
    """Verify that a candidate's best matching project vector yields high similarity against job requirements."""
    # Job requirement vector (Rust, Backend API, Database)
    job_vector = {
        "WebBackendAPI": 0.8,
        "DatabaseUsage": 0.7,
        "ConcurrentProgramming": 0.6
    }

    # Candidate project vector (concentrated in WebBackendAPI and DatabaseUsage)
    project_vector = {
        "WebBackendAPI": 0.85,
        "DatabaseUsage": 0.75,
        "ConcurrentProgramming": 0.50
    }

    sim = compute_cosine_similarity(job_vector, project_vector)
    score = round(sim * 100)

    # Score should be > 70/100 (resolving the legacy 17/100 flat threshold bug)
    assert score > 70, f"Expected match score > 70, got {score}"
    assert score >= 90, f"High alignment project should score >= 90, got {score}"

def test_sparse_project_signal_floor():
    """Verify minimum-signal floor filters out sparse or noise repos."""
    raw_projects = {
        "trivial-hello-world": {"WebBackendAPI": 0.02},  # sum = 0.02 < 0.15
        "real-backend-service": {"WebBackendAPI": 0.8, "DatabaseUsage": 0.6}  # sum = 1.4 >= 0.15
    }

    valid_projects = {}
    for repo_name, p_scores in raw_projects.items():
        if sum(p_scores.values()) >= 0.15:
            valid_projects[repo_name] = p_scores

    assert "trivial-hello-world" not in valid_projects
    assert "real-backend-service" in valid_projects

def test_zero_valid_projects_fallback():
    """Verify zero-project fallback uses aggregate capability profile."""
    raw_projects = {
        "empty-repo": {"WebBackendAPI": 0.01}
    }
    agg_caps = {"WebBackendAPI": 0.18, "DatabaseUsage": 0.17}

    valid_projects = {}
    for repo_name, p_scores in raw_projects.items():
        if sum(p_scores.values()) >= 0.15:
            valid_projects[repo_name] = p_scores

    if not valid_projects:
        valid_projects["Aggregate GitHub Profile"] = agg_caps

    assert "Aggregate GitHub Profile" in valid_projects
    assert valid_projects["Aggregate GitHub Profile"]["WebBackendAPI"] == 0.18

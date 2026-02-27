document.addEventListener('DOMContentLoaded', () => {
    // UI Elements
    const navLinks = document.querySelectorAll('.nav-links li');
    const viewTitle = document.getElementById('view-title');
    const viewDesc = document.getElementById('view-desc');
    const runBtn = document.getElementById('run-btn');
    const terminalOutput = document.getElementById('terminal-output');
    const loadingOverlay = document.getElementById('loading-overlay');

    // Input Groups
    const groupUsername = document.getElementById('group-username');
    const groupJob = document.getElementById('group-job');
    const groupProject = document.getElementById('group-project');
    const groupQuery = document.getElementById('group-query');

    // Inputs
    const inputUsername = document.getElementById('input-username');
    const inputJob = document.getElementById('input-job');
    const inputProject = document.getElementById('input-project');
    const inputQuery = document.getElementById('input-query');

    // State
    let currentView = 'evaluate';

    // View Configurations
    const viewConfigs = {
        'evaluate': {
            title: 'Evaluating Job-Fit',
            desc: 'Compare a deterministic capability vector against a structured job description.',
            showInputs: ['username', 'job'],
            endpoint: '/api/evaluate-fit',
            payload: () => ({ username: inputUsername.value, job_desc: inputJob.value }),
            validate: () => inputUsername.value && inputJob.value
        },
        'team': {
            title: 'Team Builder',
            desc: 'Compose a complementary engineering team strictly mapped to project requirements.',
            showInputs: ['project'],
            endpoint: '/api/build-team',
            payload: () => ({ project: inputProject.value }),
            validate: () => inputProject.value
        },
        'profile': {
            title: 'Intelligence Profile',
            desc: 'Translate capability signals into a human-readable executive summary.',
            showInputs: ['username'],
            endpoint: '/api/profile',
            payload: () => ({ username: inputUsername.value }),
            validate: () => inputUsername.value
        },
        'growth': {
            title: 'Career Growth Plan',
            desc: 'Identify specific knowledge gaps relative to similarity peers to map the next step.',
            showInputs: ['username'],
            endpoint: '/api/growth-plan',
            payload: () => ({ username: inputUsername.value }),
            validate: () => inputUsername.value
        },
        'interview': {
            title: 'AI Technical Interviewer',
            desc: 'Dynamically generate hyper-specific technical interview questions grounded in actual repo signals.',
            showInputs: ['username'],
            endpoint: '/api/generate-interview',
            payload: () => ({ username: inputUsername.value }),
            validate: () => inputUsername.value
        },
        'nl-search': {
            title: 'Natural Language Search',
            desc: 'Perform complex structural TF-IDF queries via zero-shot natural language.',
            showInputs: ['query'],
            endpoint: '/api/nl-search',
            payload: () => ({ query: inputQuery.value }),
            validate: () => inputQuery.value
        }
    };

    // Navigation Logic
    navLinks.forEach(link => {
        link.addEventListener('click', () => {
            // Update Active State
            navLinks.forEach(l => l.classList.remove('active'));
            link.classList.add('active');

            // Set Current View
            currentView = link.getAttribute('data-view');
            const config = viewConfigs[currentView];

            // Update Headers
            viewTitle.textContent = config.title;
            viewDesc.textContent = config.desc;

            // Toggle Inputs
            groupUsername.style.display = config.showInputs.includes('username') ? 'flex' : 'none';
            groupJob.style.display = config.showInputs.includes('job') ? 'flex' : 'none';
            groupProject.style.display = config.showInputs.includes('project') ? 'flex' : 'none';
            groupQuery.style.display = config.showInputs.includes('query') ? 'flex' : 'none';
        });
    });

    // Run Pipeline Logic
    runBtn.addEventListener('click', async () => {
        const config = viewConfigs[currentView];

        if (!config.validate()) {
            alert('Please fill out all required fields.');
            return;
        }

        // UI Loading State
        loadingOverlay.classList.remove('hidden');
        runBtn.disabled = true;
        terminalOutput.innerHTML = '';

        try {
            const response = await fetch(config.endpoint, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(config.payload())
            });

            const data = await response.json();

            // Render Output
            if (data.success) {
                terminalOutput.textContent = data.output;
            } else {
                terminalOutput.textContent = `[ERROR]\n\n${data.error}`;
            }

            // Fetch and render capabilities if username was provided and view is Intelligence Profile
            const capContainer = document.getElementById('capabilities-container');
            const capTags = document.getElementById('capabilities-tags');
            
            if (currentView === 'profile' && inputUsername.value) {
                try {
                    const capRes = await fetch('/api/capabilities', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ username: inputUsername.value })
                    });
                    const capData = await capRes.json();
                    
                    if (capData.success && capData.capabilities && capData.capabilities.capabilities) {
                        capTags.innerHTML = '';
                        const caps = capData.capabilities.capabilities;
                        const sortedCaps = Object.entries(caps).sort((a, b) => b[1] - a[1]);
                        
                        sortedCaps.forEach(([name, score]) => {
                            const badge = document.createElement('div');
                            badge.className = 'cap-badge';
                            badge.innerHTML = `<i class="fa-solid fa-code"></i> ${name} <span class="score">${score.toFixed(2)}</span>`;
                            capTags.appendChild(badge);
                        });
                        
                        capContainer.style.display = 'flex';
                    } else {
                        capContainer.style.display = 'none';
                    }
                } catch (e) {
                    console.error("Failed to load capabilities", e);
                    capContainer.style.display = 'none';
                }
            } else {
                capContainer.style.display = 'none';
            }

        } catch (err) {
            terminalOutput.textContent = `[NETWORK ERROR]\n\nFailed to reach the Flask backend. Is 'python app.py' running?`;
        } finally {
            // Restore UI
            loadingOverlay.classList.add('hidden');
            runBtn.disabled = false;
        }
    });
});

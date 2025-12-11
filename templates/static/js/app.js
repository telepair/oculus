// Switch module visibility and update nav active state
function switchModule(moduleId) {
    // Hide all modules
    document.querySelectorAll('main > section').forEach(section => {
        section.classList.add('hidden');
    });
    
    // Show target module
    const target = document.getElementById(moduleId);
    if (target) {
        target.classList.remove('hidden');
    }
    
    // Update nav active state
    document.querySelectorAll('.nav-item > div').forEach(item => {
        const isActive = item.getAttribute('onclick')?.includes(moduleId);
        if (isActive) {
            item.classList.add('bg-gradient-to-r', 'from-blue-900/50', 'to-purple-900/50', 'text-blue-400', 'shadow-lg', 'shadow-blue-500/30');
            item.classList.remove('text-neutral-400', 'hover:bg-neutral-800', 'hover:text-blue-400', 'hover:translate-x-1');
        } else {
            item.classList.remove('bg-gradient-to-r', 'from-blue-900/50', 'to-purple-900/50', 'text-blue-400', 'shadow-lg', 'shadow-blue-500/30');
            item.classList.add('text-neutral-400', 'hover:bg-neutral-800', 'hover:text-blue-400', 'hover:translate-x-1');
        }
    });
}

// Refresh metrics with current filter values
function refreshMetrics() {
    const category = document.getElementById('metric-category').value;
    const symbol = document.getElementById('metric-symbol').value;
    const limit = document.getElementById('metric-limit').value;
    const order = document.getElementById('metric-order').value;

    let url = `/api/metrics?limit=${limit}&order=${order}`;
    if (category) url += `&category=${category}`;
    if (symbol) url += `&symbol=${encodeURIComponent(symbol)}`;

    htmx.ajax('GET', url, { target: '#metrics-data', swap: 'innerHTML' });
}

// Refresh events with current filter values
function refreshEvents() {
    const source = document.getElementById('event-source').value;
    const kind = document.getElementById('event-kind').value;
    const severity = document.getElementById('event-severity').value;
    const limit = document.getElementById('event-limit').value;
    const order = document.getElementById('event-order').value;

    let url = `/api/events?limit=${limit}&order=${order}`;
    if (source) url += `&source=${encodeURIComponent(source)}`;
    if (kind) url += `&kind=${kind}`;
    if (severity) url += `&severity=${severity}`;

    htmx.ajax('GET', url, { target: '#events-data', swap: 'innerHTML' });
}

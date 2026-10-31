package main

// applyDisplayRemoteControls enriches a private display snapshot, preserving the watcher's raw baseline.
func applyDisplayRemoteControls(snap *Snapshot, busDir string, remoteControls bool) *Snapshot {
	view := *snap
	view.All = make(map[string]*Task, len(snap.All))
	for id, task := range snap.All {
		taskCopy := *task
		view.All[id] = &taskCopy
	}
	view.Tasks = make([]*Task, len(snap.Tasks))
	for i, task := range snap.Tasks {
		view.Tasks[i] = view.All[task.ID]
	}
	snap = &view
	for _, task := range snap.All {
		task.CanControl = remoteControls && task.Node != ""
		if task.CanControl {
			task.Round = mirrorQuestionRound(task.Dir)
		}
	}
	if remoteControls {
		jobs, err := readWebJobs(busDir)
		if err == nil {
			for _, job := range jobs.Jobs {
				task := snap.All[job.TaskID+"@"+job.Node]
				// Reports can precede artifact sync; the accepted assignment determines answer eligibility.
				if task != nil && task.CanControl && task.JobID == job.Request.ID && job.State == "waiting" && task.Round > 0 {
					task.Status = "waiting"
				}
			}
		} else {
			warnOnce("control-jobs:"+busDir, LogBusStateUnreadable, storageErr(err, "cannot read remote control assignments"), "dir", busDir)
		}
	}
	return snap
}

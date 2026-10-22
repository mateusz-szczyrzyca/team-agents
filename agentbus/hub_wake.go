// Qualified member wakeups are journaled on the hub while the caller owns bus.lock.
package main

import (
	"strings"
	"time"
)

// wakeMemberLocked preserves local wake hooks and routes qualified names through their owning node.
// The caller holds bus.lock. A failed remote side effect leaves the member sleeping and its messages unread.
func (b *Bus) wakeMemberLocked(team *Team, name string) bool {
	member := team.Members[name]
	if member == nil {
		return false
	}
	base, instance, qualified := strings.Cut(name, "@")
	if !qualified {
		wakeFn(team, member.Task)
		return true
	}
	if err := b.queueMemberWakeLocked(team.ID, base, instance, member.Task); err != nil {
		if member.State == MemberWaking {
			member.State = MemberSleeping
		}
		warnSideEffect("queue remote member wake", err, "member", name, "task", member.Task)
		return false
	}
	return true
}

// queueMemberWakeLocked validates the current manifest and appends a wake without reacquiring bus.lock.
// Pending wake requests for the same task/member coalesce; an unread message remains the source of truth.
func (b *Bus) queueMemberWakeLocked(team, base, instance, task string) *CommandEnqueueError {
	if !validInstanceName(instance) || !validNodeTaskID(base) || !validNodeTaskID(task) {
		return opErr(CommandEnqueueInvalidParams, "remote wake requires a valid qualified member and task")
	}
	registry := &HubInstanceRegistry{Bus: b}
	state, serr := registry.load()
	if serr != nil {
		return fromStorage(CommandEnqueueStorage, serr)
	}
	record := state.Instances[instance]
	if record == nil || !registry.now().Before(record.Lease.ExpiresAt.Time) {
		return opErr(CommandEnqueueInstanceUnreachable, "instance %q is unreachable; member stays sleeping", instance)
	}
	reported, exists := record.Manifest.CurrentState.Tasks[task]
	if !exists || reported.Agent != base || reported.Team != team {
		return opErr(CommandEnqueueInvalidParams, "instance %q has not reported task %q for this team member", instance, task)
	}
	store, serr := b.loadCommandStore()
	if serr != nil {
		return fromStorage(CommandEnqueueStorage, serr)
	}
	queue := store.queue(instance)
	member := base + "@" + instance
	for _, pending := range queue.Pending {
		if pending.Kind == CommandWake && pending.Task == task && pending.Params["member"] == member {
			return nil
		}
	}
	if len(queue.Pending) >= commandQueueLimit {
		return opErr(CommandEnqueueQueueFull, "instance %q has too many pending commands; member stays sleeping", instance)
	}
	queue.Next++
	queue.Pending = append(queue.Pending, Command{Seq: queue.Next, Kind: CommandWake, Task: task,
		Params: map[string]string{"member": member, "reason": "unread team message or assigned board work"}, EnqueuedAt: time.Now().UTC()})
	if serr := b.saveCommandStore(store); serr != nil {
		return fromStorage(CommandEnqueueStorage, serr)
	}
	logInfo(LogCommandQueued, "instance", instance, "seq", queue.Next, "kind", CommandWake, "task", task)
	return nil
}

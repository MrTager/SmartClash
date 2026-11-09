#include "CProcessIdentity.h"
#include <libproc.h>
#include <sys/proc_info.h>
#include <errno.h>
#include <poll.h>
#include <string.h>
#include <sys/event.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <sys/time.h>
#include <unistd.h>
#include <fcntl.h>
#include <signal.h>
#include <stdlib.h>
#include <time.h>
int sc_process_identity(int32_t pid, uint32_t *uid, uint64_t *seconds, uint64_t *micros, char *path, uint32_t capacity) {
    if (pid <= 1) return -1;
    struct proc_bsdinfo before = {0}, after = {0};
    errno = 0;
    if (proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &before, sizeof(before)) != sizeof(before))
        return errno == ESRCH ? 0 : -1;
    if (proc_pidpath(pid, path, capacity) <= 0) return -1;
    if (proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &after, sizeof(after)) != sizeof(after)) return -1;
    if (before.pbi_pid != after.pbi_pid || before.pbi_uid != after.pbi_uid ||
        before.pbi_start_tvsec != after.pbi_start_tvsec || before.pbi_start_tvusec != after.pbi_start_tvusec) return -1;
    *uid = before.pbi_uid; *seconds = before.pbi_start_tvsec; *micros = before.pbi_start_tvusec;
    return *seconds > 0 ? 1 : -1;
}
int sc_readable(int fd, int milliseconds) {
    struct pollfd item = { .fd = fd, .events = POLLIN | POLLHUP };
    return poll(&item, 1, milliseconds);
}

struct sc_reply { int32_t pid, running, code; };
static int exchange(int fd, char command, struct sc_reply *reply) {
    if (send(fd, &command, 1, 0) != 1 || sc_readable(fd, 4000) <= 0) return -1;
    return recv(fd, reply, sizeof(*reply), MSG_WAITALL) == sizeof(*reply) ? 0 : -1;
}
int sc_supervised_release(int fd) { struct sc_reply r; return exchange(fd, 'R', &r) == 0 && r.code == 0 ? 0 : -1; }
int sc_supervised_status(int fd, int32_t *running, int32_t *code) {
    struct sc_reply r; if (exchange(fd, '?', &r)) return -1;
    *running = r.running; *code = r.code; return 0;
}
int sc_supervised_stop(int fd) {
    struct sc_reply r; return exchange(fd, 'X', &r) == 0 && !r.running ? 0 : -1;
}
int sc_peer_pid(int fd, int32_t *pid) {
    socklen_t size = sizeof(*pid); return getsockopt(fd, SOL_LOCAL, LOCAL_PEERPID, pid, &size);
}
static void reap_core(pid_t child, int *status, int *running) {
    if (*running && waitpid(child, status, WNOHANG) == child) *running = 0;
}
static void stop_core(pid_t child, int *status, int *running) {
    // 未 waitpid 的直接子进程 PID 不会复用；绝不从日志读取 PID 后发信号。
    reap_core(child, status, running);
    if (!*running) return;
    kill(child, SIGTERM);
    struct timespec delay = {0, 10000000};
    for (int i = 0; i < 150 && *running; ++i) { nanosleep(&delay, NULL); reap_core(child, status, running); }
    if (*running) { kill(child, SIGKILL); while (waitpid(child, status, 0) < 0 && errno == EINTR) {} *running = 0; }
}
int sc_supervised_spawn(const char *exe, char *const argv[], const char *directory,
                        int log_fd, int lock_fd, int32_t *monitor_pid, int32_t *core_pid, int *channel) {
    int sockets[2]; if (socketpair(AF_UNIX, SOCK_STREAM, 0, sockets)) return -1;
    int one = 1; setsockopt(sockets[0], SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
    setsockopt(sockets[1], SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
    pid_t owner = getpid(), monitor = fork();
    if (monitor < 0) { close(sockets[0]); close(sockets[1]); return -1; }
    if (monitor == 0) {
        close(sockets[0]);
        int kq = kqueue(); struct kevent event;
        EV_SET(&event, owner, EVFILT_PROC, EV_ADD | EV_ENABLE, NOTE_EXIT, 0, NULL);
        if (kq < 0 || kevent(kq, &event, 1, NULL, 0, NULL) < 0 || getppid() != owner) _exit(71);
        EV_SET(&event, sockets[1], EVFILT_READ, EV_ADD | EV_ENABLE, 0, 0, NULL);
        if (kevent(kq, &event, 1, NULL, 0, NULL) < 0) _exit(71);
        // 保留 guardian 锁，且关闭 GUI 管道，GUI 死亡后不会被孙进程持有写端阻止 EOF。
        int limit = getdtablesize();
        for (int fd = 0; fd < limit; ++fd) if (fd != sockets[1] && fd != kq && fd != lock_fd && fd != log_fd) close(fd);
        int gate[2]; if (pipe(gate)) _exit(71);
        pid_t child = fork();
        if (child < 0) _exit(71);
        if (child == 0) {
            close(gate[1]); close(sockets[1]); close(kq); close(lock_fd);
            char release = 0;
            if (read(gate[0], &release, 1) != 1 || release != 'R') _exit(72);
            close(gate[0]);
            if (chdir(directory) || dup2(log_fd, STDOUT_FILENO) < 0 || dup2(log_fd, STDERR_FILENO) < 0) _exit(73);
            close(log_fd); int null_fd = open("/dev/null", O_RDONLY); if (null_fd >= 0) { dup2(null_fd, 0); if (null_fd != 0) close(null_fd); }
            char *env[] = {"PATH=/usr/bin:/bin", "LC_ALL=C", NULL};
            execve(exe, argv, env); _exit(74);
        }
        close(gate[0]); close(log_fd);
        struct sc_reply reply = {child, 1, 0};
        if (send(sockets[1], &reply, sizeof(reply), 0) != sizeof(reply)) { kill(child, SIGKILL); _exit(75); }
        int status = 0, running = 1, released = 0;
        while (1) {
            struct timespec timeout = {1, 0};
            int n = kevent(kq, NULL, 0, &event, 1, &timeout);
            if (n < 0 && errno == EINTR) continue;
            if (n < 0 || getppid() != owner || (n > 0 && event.filter == EVFILT_PROC)) break;
            if (n == 0) continue;
            char command; if (recv(sockets[1], &command, 1, 0) != 1) break;
            if (command == 'R' && !released) { released = 1; reply.code = write(gate[1], "R", 1) == 1 ? 0 : -1; close(gate[1]); }
            else if (command == 'X') { if (!released) { close(gate[1]); released = 1; } stop_core(child, &status, &running); }
            else if (command != '?') break;
            reap_core(child, &status, &running);
            reply.running = running; if (!running) reply.code = WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
            if (send(sockets[1], &reply, sizeof(reply), 0) != sizeof(reply)) break;
            if (command == 'X') _exit(0);
        }
        if (!released) close(gate[1]);
        stop_core(child, &status, &running); _exit(0);
    }
    close(sockets[1]);
    struct sc_reply reply;
    if (sc_readable(sockets[0], 4000) <= 0 || recv(sockets[0], &reply, sizeof(reply), MSG_WAITALL) != sizeof(reply)) { close(sockets[0]); return -1; }
    fcntl(sockets[0], F_SETFD, FD_CLOEXEC);
    *monitor_pid = monitor; *core_pid = reply.pid; *channel = sockets[0]; return 0;
}

#ifndef SMARTCLASH_PROCESS_IDENTITY_H
#define SMARTCLASH_PROCESS_IDENTITY_H
#include <stdint.h>
#include <stddef.h>
// 1 为已核实，0 为确定不存在，-1 为查询未知。
int sc_process_identity(int32_t pid, uint32_t *uid, uint64_t *seconds, uint64_t *micros, char *path, uint32_t capacity);
int sc_readable(int fd, int milliseconds);
// 每个内核由独立的同用户监督进程持有；exec 前已经注册 guardian 的 NOTE_EXIT。
int sc_supervised_spawn(const char *executable, char *const argv[], const char *directory,
                        int log_fd, int lock_fd, int32_t *monitor_pid, int32_t *core_pid, int *channel);
int sc_supervised_release(int channel);
int sc_supervised_status(int channel, int32_t *running, int32_t *code);
int sc_supervised_stop(int channel);
int sc_peer_pid(int socket_fd, int32_t *pid);
#endif

#!/usr/bin/env ruby
# Isolated, non-TUN diagnostics. Never changes the OS proxy or prints credentials.
require "json"
require "net/http"
require "tmpdir"
require "socket"
require "securerandom"
require "yaml"
require "open3"

def free_port
  socket = TCPServer.new("127.0.0.1", 0)
  port = socket.addr[1]
  socket.close
  port
end

def request(port, secret, path, timeout = 8, method = "GET", body = nil)
  http = Net::HTTP.new("127.0.0.1", port, nil)
  http.open_timeout = 2
  http.read_timeout = timeout
  req = method == "PUT" ? Net::HTTP::Put.new(path) : Net::HTTP::Get.new(path)
  req["Authorization"] = "Bearer #{secret}"
  if body
    req["Content-Type"] = "application/json"
    req.body = JSON.generate(body)
  end
  response = http.request(req)
  [response.code.to_i, response.body.to_s.empty? ? {} : JSON.parse(response.body)]
end

begin
  File.umask(0077)
  root = File.expand_path("..", __dir__)
  data_root = File.join(Dir.home, "Library/Application Support/SmartClash")
  config = JSON.parse(File.read(File.join(data_root, "config.json")))
  rows = config.fetch("proxies").reject { |node| node.key?("dialer-proxy") }
  # Interleave sources to avoid spending the entire initial budget on one expired source.
  prefs = JSON.parse(File.read(File.join(data_root, "preferences.json")))
  pools = prefs.fetch("sources").map do |source|
    names = YAML.safe_load(File.read(File.join(data_root, "subscription-#{source.fetch("id")}.yaml")), [], [], false)
      .fetch("proxies").map { |node| node.fetch("name") }
    rows.select { |node| names.any? { |name| node.fetch("name").start_with?(name + " · ") } }
  end
  ordered = []
  until pools.all?(&:empty?)
    pools.each { |pool| ordered << pool.shift unless pool.empty? }
  end
  ordered = ordered.uniq { |node| node.fetch("name") }
  if ARGV.include?("--websites")
    samples = JSON.parse(File.read(File.join(data_root, "latencies.json")))
    good = samples.select { |_, sample| sample["milliseconds"].is_a?(Numeric) }.sort_by { |_, sample| sample["milliseconds"] }
    ordered = good.map { |id, _| rows.find { |node| node.fetch("name").end_with?(" · #{id[0,12]}") } }.compact.first(3)
    raise "no measured candidates" if ordered.empty?
  end
  limit = ARGV.include?("--all") ? rows.length : 28
  ordered = ordered.first(limit)
  controller = free_port
  config["mixed-port"] = free_port
  config["external-controller"] = "127.0.0.1:#{controller}"
  config.delete("listeners")
  config["tun"] = { "enable" => false }
  config["proxies"] = rows
  config["proxy-groups"] = [{ "name" => "SMART", "type" => "select", "proxies" => rows.map { |n| n.fetch("name") } }]
  config["dns"]["listen"] = "127.0.0.1:#{free_port}"
  config["secret"] = SecureRandom.hex(24)
  summary = nil
  Dir.mktmpdir("smartclash-network-check-") do |directory|
    filename = File.join(directory, "config.json")
    File.write(filename, JSON.generate(config))
    logfile = File.join(directory, "core.log")
    pid = Process.spawn(File.join(root, "Resources/mihomo"), "-d", directory, "-f", filename,
      :out => logfile, :err => logfile)
    begin
      ready = false
      40.times do
        begin
          ready = request(controller, config["secret"], "/version")[0] == 200
          break if ready
        rescue StandardError
          sleep 0.1
        end
      end
      raise "test core did not start" unless ready
      queue = Queue.new
      ordered.each_with_index { |node, index| queue << [node, index] }
      results = []
      mutex = Mutex.new
      workers = 6.times.map do
        Thread.new do
          loop do
            item = queue.pop(true) rescue nil
            break unless item
            node, index = item
            name = URI.encode_www_form_component(node.fetch("name")).gsub("+", "%20")
            query = URI.encode_www_form("url" => "https://cp.cloudflare.com/generate_204", "timeout" => "5000", "expected" => "204")
            result = { "index" => index, "type" => node["type"] }
            begin
              code, response = request(controller, config["secret"], "/proxies/#{name}/delay?#{query}")
              result["ok"] = code == 200 && response["delay"].is_a?(Numeric)
              result["delay"] = response["delay"] if result["ok"]
            rescue StandardError
              result["ok"] = false
            end
            mutex.synchronize { results << result }
          end
        end
      end
      workers.each(&:join)
      websites = []
      if ARGV.include?("--websites")
        ordered.each_with_index do |node, index|
          request(controller, config["secret"], "/proxies/SMART", 8, "PUT", { "name" => node.fetch("name") })
          { "apple" => "https://www.apple.com/library/test/success.html",
            "https_204" => "https://cp.cloudflare.com/generate_204",
            "openai" => "https://api.openai.com/v1/models" }.each do |label, target|
            stdout, _, status = Open3.capture3("/usr/bin/curl", "--silent", "--show-error", "--noproxy", "", "--proxy",
              "http://127.0.0.1:#{config["mixed-port"]}", "--connect-timeout", "3", "--max-time", "7", "--max-filesize", "131072",
              "--write-out", "\n%{http_code}", target)
            body, _, code = stdout.rpartition("\n")
            item = { candidate: index + 1, target: label, http: code.to_i, transfer_ok: status.success? }
            if label == "openai"
              parsed = JSON.parse(body) rescue {}
              message = parsed.dig("error", "message").to_s.downcase
              item[:api_reachable] = code.to_i == 401 && (message.include?("api key") || message.include?("authentication"))
            end
            websites << item
          end
        end
      end
      log = File.read(logfile)
      summary = { tested: results.length, passed: results.count { |r| r["ok"] },
        best_ms: results.select { |r| r["ok"] }.map { |r| r["delay"] }.min,
        protocol_results: results.group_by { |r| r["type"] }.transform_values { |items| { tested: items.length, passed: items.count { |r| r["ok"] } } },
        dns_errors: log.scan(/dns resolve failed/).length, network_unreachable: log.scan(/network is unreachable/).length }
      summary[:websites] = websites unless websites.empty?
    ensure
      begin
        Process.kill("TERM", pid)
        Process.wait(pid)
      rescue Errno::ESRCH, Errno::ECHILD
      end
    end
  end
  puts JSON.pretty_generate(summary)
rescue StandardError => error
  warn "Isolated diagnostic failed (#{error.class}); sensitive details hidden."
  exit 1
end

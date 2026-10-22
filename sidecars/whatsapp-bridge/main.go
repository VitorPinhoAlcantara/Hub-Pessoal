package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
	"sync"
)

const maxLineSize = 16 * 1024 * 1024

type output struct {
	mu  sync.Mutex
	enc *json.Encoder
}

func newOutput(w io.Writer) *output {
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	return &output{enc: enc}
}

func (o *output) write(value any) {
	o.mu.Lock()
	defer o.mu.Unlock()
	if err := o.enc.Encode(value); err != nil {
		log.Printf("falha ao escrever na saída: %v", err)
	}
}

func main() {
	log.SetOutput(os.Stderr)
	os.Exit(run(os.Args[1:], os.Stdin, os.Stdout))
}

func run(args []string, in io.Reader, stdout io.Writer) int {
	flags := flag.NewFlagSet("whatsapp-bridge", flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	dataDir := flags.String("dados", "", "pasta onde a sessão do WhatsApp é guardada")
	if err := flags.Parse(args); err != nil {
		return 2
	}
	if *dataDir == "" {
		fmt.Fprintln(os.Stderr, "uso: whatsapp-bridge --dados <pasta>")
		return 2
	}

	out := newOutput(stdout)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	b, err := newBridge(ctx, *dataDir, out)
	if err != nil {
		log.Printf("falha ao iniciar: %v", err)
		out.write(errorEvent(err.Error()))
		return 1
	}

	scanner := bufio.NewScanner(in)
	scanner.Buffer(make([]byte, 0, 64*1024), maxLineSize)
	for scanner.Scan() {
		line := scanner.Bytes()
		cmd, err := decodeCommand(line)
		if errors.Is(err, errEmptyCommand) {
			continue
		}
		if err != nil {
			if cmd.ID != 0 {
				out.write(errorResponse(cmd.ID, "Comando inválido: "+err.Error()))
			} else {
				out.write(errorEvent("Comando inválido: " + err.Error()))
			}
			continue
		}
		if cmd.Cmd == CmdShutdown {
			b.shutdown()
			out.write(okResponse(cmd.ID, nil))
			return 0
		}
		go b.handle(cmd)
	}
	if err := scanner.Err(); err != nil {
		log.Printf("falha ao ler a entrada: %v", err)
	}
	b.shutdown()
	return 0
}

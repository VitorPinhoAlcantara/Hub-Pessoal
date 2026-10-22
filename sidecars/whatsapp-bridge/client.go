package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

const sessionFileName = "whatsapp-sessao.db"

var errNotConnected = errors.New("o WhatsApp não está conectado")

type stderrLogger struct {
	module string
	debug  bool
	out    *log.Logger
}

func newStderrLogger(module string, debug bool) waLog.Logger {
	return &stderrLogger{module: module, debug: debug, out: log.New(os.Stderr, "", log.LstdFlags)}
}

func (l *stderrLogger) write(level, msg string, args ...any) {
	l.out.Printf("[%s %s] %s", l.module, level, fmt.Sprintf(msg, args...))
}

func (l *stderrLogger) Warnf(msg string, args ...any)  { l.write("WARN", msg, args...) }
func (l *stderrLogger) Errorf(msg string, args ...any) { l.write("ERROR", msg, args...) }
func (l *stderrLogger) Infof(msg string, args ...any)  { l.write("INFO", msg, args...) }

func (l *stderrLogger) Debugf(msg string, args ...any) {
	if l.debug {
		l.write("DEBUG", msg, args...)
	}
}

func (l *stderrLogger) Sub(module string) waLog.Logger {
	return &stderrLogger{module: l.module + "/" + module, debug: l.debug, out: l.out}
}

type bridge struct {
	ctx       context.Context
	container *sqlstore.Container
	out       *output
	log       waLog.Logger
	mu        sync.Mutex
	client    atomic.Pointer[whatsmeow.Client]
}

func sessionDSN(dataDir string) string {
	path := filepath.Join(dataDir, sessionFileName)
	return path + "?_pragma=foreign_keys(1)&_pragma=busy_timeout(10000)&_pragma=journal_mode(WAL)"
}

func newBridge(ctx context.Context, dataDir string, out *output) (*bridge, error) {
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return nil, fmt.Errorf("não foi possível criar a pasta de dados: %w", err)
	}
	logger := newStderrLogger("whatsapp", os.Getenv("WHATSAPP_BRIDGE_DEBUG") != "")
	container, err := sqlstore.New(ctx, "sqlite", sessionDSN(dataDir), logger.Sub("Banco"))
	if err != nil {
		return nil, fmt.Errorf("não foi possível abrir a sessão do WhatsApp: %w", err)
	}
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		return nil, fmt.Errorf("não foi possível ler a sessão do WhatsApp: %w", err)
	}
	b := &bridge{ctx: ctx, container: container, out: out, log: logger}
	b.client.Store(b.newClient(device))
	return b, nil
}

func (b *bridge) newClient(device *store.Device) *whatsmeow.Client {
	client := whatsmeow.NewClient(device, b.log.Sub("Cliente"))
	client.AddEventHandler(func(evt any) {
		b.handleEvent(client, evt)
	})
	return client
}

func (b *bridge) current() *whatsmeow.Client {
	return b.client.Load()
}

func (b *bridge) handle(cmd Command) {
	switch cmd.Cmd {
	case CmdLogin:
		b.out.write(okResponse(cmd.ID, nil))
		go func() {
			if err := b.login(); err != nil {
				b.log.Errorf("falha ao conectar: %v", err)
				b.out.write(errorEvent("Não foi possível conectar ao WhatsApp: " + err.Error()))
			}
		}()
	case CmdListConversations:
		conversations, err := b.listConversations()
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível listar as conversas: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, conversations))
	case CmdListMessages:
		b.out.write(okResponse(cmd.ID, []Message{}))
	case CmdSend:
		msg, err := b.send(cmd.JID, cmd.Text)
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível enviar a mensagem: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, msg))
	default:
		b.out.write(errorResponse(cmd.ID, "Comando desconhecido: "+cmd.Cmd))
	}
}

func (b *bridge) login() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	client := b.current()
	if client.IsConnected() {
		if client.IsLoggedIn() && client.Store.ID != nil {
			b.out.write(connectedEvent(client.Store.ID.ToNonAD().String()))
		}
		return nil
	}
	if client.Store.ID == nil {
		qr, err := client.GetQRChannel(b.ctx)
		if err != nil {
			return err
		}
		go b.watchQR(qr)
	}
	return client.Connect()
}

func (b *bridge) watchQR(qr <-chan whatsmeow.QRChannelItem) {
	for item := range qr {
		switch item.Event {
		case whatsmeow.QRChannelEventCode:
			b.out.write(qrEvent(item.Code))
		case whatsmeow.QRChannelSuccess.Event:
			b.log.Infof("pareamento concluído")
		case whatsmeow.QRChannelTimeout.Event:
			b.out.write(errorEvent("O código QR expirou. Tente entrar de novo."))
		case whatsmeow.QRChannelEventError:
			detail := "erro desconhecido"
			if item.Error != nil {
				detail = item.Error.Error()
			}
			b.out.write(errorEvent("Falha no pareamento: " + detail))
		case whatsmeow.QRChannelClientOutdated.Event:
			b.out.write(errorEvent("Esta versão do cliente do WhatsApp está desatualizada."))
		default:
			if strings.HasPrefix(item.Event, "passkey") {
				b.log.Warnf("evento de pareamento não suportado: %s", item.Event)
				continue
			}
			b.out.write(errorEvent("Falha no pareamento: " + item.Event))
		}
	}
}

func (b *bridge) listConversations() ([]Conversation, error) {
	client := b.current()
	if !client.IsLoggedIn() {
		return nil, errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 30*time.Second)
	defer cancel()
	contacts, err := client.Store.Contacts.GetAllContacts(ctx)
	if err != nil {
		return nil, err
	}
	groups, err := client.GetJoinedGroups(ctx)
	if err != nil {
		b.log.Warnf("falha ao obter grupos: %v", err)
		groups = nil
	}
	return buildConversations(contacts, groups), nil
}

func (b *bridge) send(jid, text string) (Message, error) {
	if strings.TrimSpace(jid) == "" {
		return Message{}, errors.New("conversa não informada")
	}
	if strings.TrimSpace(text) == "" {
		return Message{}, errors.New("texto vazio")
	}
	target, err := types.ParseJID(jid)
	if err != nil {
		return Message{}, fmt.Errorf("conversa inválida: %w", err)
	}
	client := b.current()
	if !client.IsLoggedIn() || client.Store.ID == nil {
		return Message{}, errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 60*time.Second)
	defer cancel()
	resp, err := client.SendMessage(ctx, target, &waE2E.Message{Conversation: proto.String(text)})
	if err != nil {
		return Message{}, err
	}
	return sentMessage(string(resp.ID), target, *client.Store.ID, text, resp.Timestamp), nil
}

func (b *bridge) handleEvent(client *whatsmeow.Client, evt any) {
	if client != b.current() {
		return
	}
	switch e := evt.(type) {
	case *events.Connected:
		if client.Store.ID != nil {
			b.out.write(connectedEvent(client.Store.ID.ToNonAD().String()))
		}
	case *events.Message:
		if isRelevantMessage(e) {
			b.out.write(messageEvent(messageFromEvent(e), false))
		}
	case *events.HistorySync:
		b.handleHistory(client, e)
	case *events.LoggedOut:
		b.out.write(disconnectedEvent(ReasonSessionEnded))
		go b.resetSession(client)
	case *events.Disconnected:
		b.out.write(disconnectedEvent(ReasonConnectionLost))
	}
}

func (b *bridge) handleHistory(client *whatsmeow.Client, evt *events.HistorySync) {
	if evt.Data == nil {
		return
	}
	for _, conv := range evt.Data.GetConversations() {
		chat, err := types.ParseJID(conv.GetID())
		if err != nil {
			b.log.Warnf("conversa do histórico inválida %q: %v", conv.GetID(), err)
			continue
		}
		for _, item := range conv.GetMessages() {
			web := item.GetMessage()
			if web == nil {
				continue
			}
			parsed, err := client.ParseWebMessage(chat, web)
			if err != nil {
				b.log.Debugf("mensagem do histórico ignorada: %v", err)
				continue
			}
			if isRelevantMessage(parsed) {
				b.out.write(messageEvent(messageFromEvent(parsed), true))
			}
		}
	}
}

func (b *bridge) resetSession(old *whatsmeow.Client) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if old != b.current() {
		return
	}
	old.Disconnect()
	if err := old.Store.Delete(b.ctx); err != nil {
		b.log.Errorf("falha ao apagar a sessão: %v", err)
	}
	b.client.Store(b.newClient(b.container.NewDevice()))
}

func (b *bridge) shutdown() {
	b.mu.Lock()
	defer b.mu.Unlock()
	if client := b.current(); client != nil {
		client.Disconnect()
	}
	if err := b.container.Close(); err != nil {
		b.log.Warnf("falha ao fechar o banco da sessão: %v", err)
	}
}
